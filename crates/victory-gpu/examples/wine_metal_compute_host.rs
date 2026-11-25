//! Opt-in loopback server for the restricted Wine Metal compute and graphics protocols.

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
mod host {
    use std::io::{self, Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};

    use std::time::Duration;
    const MAGIC: u32 = 0x3147_4d56;
    const MAX_SHADER: usize = 4 * 1024 * 1024;
    const MAX_BUFFER: usize = 16 * 1024 * 1024;
    const MAX_MESSAGE: usize = 4096;

    fn token() -> Result<[u8; 16], String> {
        let value = std::env::var("VICTORY_WINE_METAL_TOKEN")
            .map_err(|_| "VICTORY_WINE_METAL_TOKEN must be 32 hexadecimal digits")?;
        let bytes = value.as_bytes();
        if bytes.len() != 32 || !bytes.iter().all(u8::is_ascii_hexdigit) {
            return Err("VICTORY_WINE_METAL_TOKEN must be 32 hexadecimal digits".into());
        }
        let mut token = [0; 16];
        for (i, byte) in token.iter_mut().enumerate() {
            let digit = |c: u8| match c {
                b'0'..=b'9' => c - b'0',
                b'a'..=b'f' => c - b'a' + 10,
                _ => c - b'A' + 10,
            };
            *byte = (digit(bytes[2 * i]) << 4) | digit(bytes[2 * i + 1]);
        }
        Ok(token)
    }

    fn word(bytes: &[u8]) -> u32 {
        u32::from_le_bytes(bytes.try_into().expect("four-byte word"))
    }

    fn compute_request(
        stream: &mut TcpStream,
        fields: [u32; 8],
        cache: &mut Option<(Vec<u8>, bool, victory_gpu::WineMetalCompute)>,
    ) -> Result<Vec<u8>, String> {
        let shader_len = fields[2] as usize;
        let buffer_len = fields[3] as usize;
        let count = fields[4] as usize;
        let groups = [fields[5], fields[6], fields[7]];
        let cbv_info = if fields[1] == 6 {
            let mut bytes = [0; 8];
            stream.read_exact(&mut bytes).map_err(|e| e.to_string())?;
            let slot = word(&bytes[..4]);
            let len = word(&bytes[4..]) as usize;
            if !(2..64).contains(&slot) || len == 0 || len > 65536 {
                return Err("invalid root CBV slot or length".into());
            }
            Some((slot, len))
        } else {
            None
        };
        let raw_info = if fields[1] == 7 {
            let mut bytes = [0; 20];
            stream.read_exact(&mut bytes).map_err(|e| e.to_string())?;
            let words: [usize; 5] = std::array::from_fn(|i| word(&bytes[i * 4..i * 4 + 4]) as usize);
            let [input_len, srv_offset, srv_len, uav_offset, uav_len] = words;
            if input_len == 0 || input_len > MAX_BUFFER
                || srv_len == 0 || srv_offset.checked_add(srv_len).is_none_or(|end| end > input_len)
                || uav_len == 0 || uav_offset.checked_add(uav_len).is_none_or(|end| end > buffer_len)
            {
                return Err("invalid raw buffer view".into());
            }
            Some((input_len, (srv_offset, srv_len), (uav_offset, uav_len)))
        } else {
            None
        };
        let mut shader = vec![0; shader_len];
        let mut buffer = vec![0; buffer_len];
        let mut constant_bytes = vec![0; count * 4];
        stream.read_exact(&mut shader).map_err(|e| e.to_string())?;
        stream.read_exact(&mut buffer).map_err(|e| e.to_string())?;
        stream.read_exact(&mut constant_bytes).map_err(|e| e.to_string())?;
        let mut cbv = vec![0; cbv_info.map_or(0, |(_, len)| len)];
        stream.read_exact(&mut cbv).map_err(|e| e.to_string())?;
        let mut input = vec![0; raw_info.map_or(0, |(len, _, _)| len)];
        stream.read_exact(&mut input).map_err(|e| e.to_string())?;
        let constants: Vec<u32> = constant_bytes.chunks_exact(4).map(word).collect();
        let raw = raw_info.is_some();
        if cache.as_ref().is_none_or(|(cached, cached_raw, _)| *cached != shader || *cached_raw != raw) {
            // Drop the old pipeline before constructing a replacement.
            *cache = None;
            let pipeline = if raw {
                victory_gpu::WineMetalCompute::new_raw_views(&shader)?
            } else {
                victory_gpu::WineMetalCompute::new(&shader)?
            };
            *cache = Some((shader, raw, pipeline));
        }
        let pipeline = &mut cache.as_mut().expect("pipeline was prepared").2;
        if let Some((_, srv, uav)) = raw_info {
            pipeline.dispatch_raw_views(&mut buffer, &input, srv, uav, &constants, groups)?;
        } else if let Some((slot, _)) = cbv_info {
            pipeline.dispatch_with_root_cbv(&mut buffer, &constants, groups, slot, &cbv)?;
        } else {
            pipeline.dispatch(&mut buffer, &constants, groups)?;
        }
        Ok(buffer)
    }

    fn raw_heap_request(
        stream: &mut TcpStream,
        len: usize,
        cache: &mut Option<(Vec<u8>, bool, victory_gpu::WineMetalCompute)>,
    ) -> Result<Vec<u8>, String> {
        let mut payload = vec![0; len];
        stream.read_exact(&mut payload).map_err(|e| e.to_string())?;
        let mut cursor = 0usize;
        let take = |cursor: &mut usize, count: usize| -> Result<&[u8], String> {
            let end = cursor.checked_add(count).ok_or("raw heap payload overflow")?;
            let bytes = payload.get(*cursor..end).ok_or("truncated raw heap payload")?;
            *cursor = end;
            Ok(bytes)
        };
        let header = take(&mut cursor, 32)?;
        let fields: [u32; 8] = std::array::from_fn(|i| word(&header[i * 4..i * 4 + 4]));
        let [shader_len, count, table_slot, table_base, desc_count, x, y, z] = fields;
        if shader_len == 0 || shader_len as usize > MAX_SHADER || count > 64
            || !(1..64).contains(&table_slot) || desc_count == 0 || desc_count > 16
            || [x, y, z].contains(&0)
        {
            return Err("invalid raw heap request".into());
        }
        let shader = take(&mut cursor, shader_len as usize)?.to_vec();
        let constants: Vec<u32> = take(&mut cursor, count as usize * 4)?
            .chunks_exact(4).map(word).collect();
        let mut entries = Vec::with_capacity(desc_count as usize);
        let mut response_len = 0usize;
        for _ in 0..desc_count {
            let entry = take(&mut cursor, 16)?;
            let kind = word(&entry[0..4]);
            let offset = word(&entry[4..8]) as usize;
            let length = word(&entry[8..12]) as usize;
            let size = word(&entry[12..16]) as usize;
            if !matches!(kind, 1 | 2) || size == 0 || length == 0
                || offset.checked_add(length).is_none_or(|end| end > size)
            {
                return Err("invalid raw heap descriptor".into());
            }
            if kind == 2 {
                response_len = response_len.checked_add(size).ok_or("raw heap response overflow")?;
                if response_len > 64 * 1024 * 1024 {
                    return Err("raw heap response too large".into());
                }
            }
            entries.push((kind, offset, length, take(&mut cursor, size)?.to_vec()));
        }
        if cursor != payload.len() {
            return Err("trailing raw heap payload".into());
        }
        if cache.as_ref().is_none_or(|(cached, raw, _)| *cached != shader || !*raw) {
            *cache = None;
            let pipeline = victory_gpu::WineMetalCompute::new_raw_views(&shader)?;
            *cache = Some((shader, true, pipeline));
        }
        let mut resources: Vec<_> = entries.iter_mut().map(|(kind, offset, length, bytes)|
            victory_metal::compute::RawHeapBuffer {
                bytes, offset: *offset, length: *length, writable: *kind == 2,
            }).collect();
        cache.as_mut().expect("pipeline was prepared").2.dispatch_raw_heap(
            &mut resources, table_slot, table_base, &constants, [x, y, z])?;
        let mut response = Vec::with_capacity(response_len);
        for (kind, _, _, bytes) in entries {
            if kind == 2 { response.extend_from_slice(&bytes); }
        }
        Ok(response)
    }

    fn graphics_request(stream: &mut TcpStream, fields: [u32; 8]) -> Result<Vec<u8>, String> {
        let vs_len = fields[2] as usize;
        let ps_len = fields[3] as usize;
        let width = fields[4];
        let height = fields[5];
        let vertex_count = fields[6] as usize;
        let count = fields[7] as usize;
        let image_len = (width as usize).checked_mul(height as usize)
            .and_then(|n| n.checked_mul(4)).ok_or("image size overflow")?;
        if vs_len == 0 || vs_len > MAX_SHADER || ps_len == 0 || ps_len > MAX_SHADER
            || width == 0 || height == 0 || image_len > MAX_BUFFER
            || vertex_count != 3 || count > 64
        {
            return Err("invalid graphics request dimensions or lengths".into());
        }
        let mut clear_bytes = [0; 16];
        stream.read_exact(&mut clear_bytes).map_err(|e| e.to_string())?;
        let clear = std::array::from_fn(|i| f32::from_bits(word(&clear_bytes[i * 4..i * 4 + 4])));
        let mut vertex_shader = vec![0; vs_len];
        let mut pixel_shader = vec![0; ps_len];
        let mut vertex_bytes = [0; 48];
        let mut constant_bytes = vec![0; count * 4];
        stream.read_exact(&mut vertex_shader).map_err(|e| e.to_string())?;
        stream.read_exact(&mut pixel_shader).map_err(|e| e.to_string())?;
        stream.read_exact(&mut vertex_bytes).map_err(|e| e.to_string())?;
        stream.read_exact(&mut constant_bytes).map_err(|e| e.to_string())?;
        let vertices: [[f32; 4]; 3] = std::array::from_fn(|i| {
            std::array::from_fn(|j| {
                let offset = (i * 4 + j) * 4;
                f32::from_bits(word(&vertex_bytes[offset..offset + 4]))
            })
        });
        let constants: Vec<u32> = constant_bytes.chunks_exact(4).map(word).collect();
        let mut pipeline = victory_gpu::WineMetalGraphics::new(&vertex_shader, &pixel_shader)?;
        pipeline.draw(width, height, clear, &vertices, &constants)
    }

    fn present_request(
        stream: &mut TcpStream,
        fields: [u32; 8],
        enabled: bool,
    ) -> Result<Vec<u8>, String> {
        if !enabled {
            return Err("presentation is not enabled".into());
        }
        let [_, _, width, height, format, pitch, bytes_len, flags] = fields;
        let row_bytes = (width as usize).checked_mul(4).ok_or("image size overflow")?;
        let required = (height as usize).checked_sub(1)
            .and_then(|rows| rows.checked_mul(pitch as usize))
            .and_then(|bytes| bytes.checked_add(row_bytes))
            .ok_or("image size overflow")?;
        if width == 0 || !matches!(format, 28 | 29 | 87 | 91) || flags > 1
            || (pitch as usize) < row_bytes || required != bytes_len as usize
            || required > MAX_BUFFER
        {
            return Err("invalid present request dimensions, format, or length".into());
        }
        let mut pixels = vec![0; required];
        stream.read_exact(&mut pixels).map_err(|e| e.to_string())?;
        let result = victory_metal::present::present(width, height, format, pitch, &pixels, flags == 1)
            .map_err(|e| e.to_string())?;
        let capture_len = result.capture.len();
        let expected_capture = (result.width as usize).checked_mul(result.height as usize)
            .and_then(|n| n.checked_mul(4)).ok_or("drawable size overflow")?;
        if result.width == 0 || result.height == 0
            || (flags == 1 && (capture_len != expected_capture || capture_len > MAX_BUFFER))
            || (flags == 0 && capture_len != 0)
        {
            return Err("invalid drawable capture returned by presenter".into());
        }
        let mut response = Vec::with_capacity(12 + capture_len);
        response.extend_from_slice(&result.width.to_le_bytes());
        response.extend_from_slice(&result.height.to_le_bytes());
        response.extend_from_slice(&(capture_len as u32).to_le_bytes());
        response.extend_from_slice(&result.capture);
        Ok(response)
    }

    #[derive(Debug)]
    enum Response {
        Bytes(Vec<u8>),
        Stream(Vec<victory_gpu::wine_transport::Resource>),
    }

    fn request(
        stream: &mut TcpStream,
        expected_token: &[u8; 16],
        cache: &mut Option<(Vec<u8>, bool, victory_gpu::WineMetalCompute)>,
        presentation_enabled: bool,
    ) -> Result<Response, String> {
        let mut header = [0; 32];
        stream.read_exact(&mut header).map_err(|e| e.to_string())?;
        let fields: [u32; 8] = std::array::from_fn(|i| word(&header[i * 4..i * 4 + 4]));
        if fields[0] != MAGIC || !matches!(fields[1], 1 | 2 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 | 16 | 17 | 18 | 19 | 20 | 21) {
            return Err("invalid protocol header".into());
        }
        if matches!(fields[1], 1 | 6 | 7) {
            let shader_len = fields[2] as usize;
            let buffer_len = fields[3] as usize;
            if shader_len == 0 || shader_len > MAX_SHADER || buffer_len == 0
                || buffer_len > MAX_BUFFER || fields[4] > 64 || fields[5..8].contains(&0)
            {
                return Err("invalid request lengths or dispatch dimensions".into());
            }
        }
        if matches!(fields[1], 5 | 10 | 12 | 14 | 16 | 18 | 20) && (fields[2] as usize > victory_gpu::wine_transport::MAX_PAYLOAD
            || fields[3..].iter().any(|&v| v != 0))
        {
            return Err("invalid batch request length or reserved fields".into());
        }
        if matches!(fields[1], 9 | 11 | 13 | 15 | 17 | 19 | 21) && fields[2..].iter().any(|&v| v != 0) {
            return Err("invalid stream request reserved fields".into());
        }
        if fields[1] == 8 && (fields[2] < 32 || fields[2] as usize > 64 * 1024 * 1024
            || fields[3..].iter().any(|&v| v != 0))
        {
            return Err("invalid raw heap payload length or reserved fields".into());
        }
        let mut supplied_token = [0; 16];
        stream.read_exact(&mut supplied_token).map_err(|e| e.to_string())?;
        if supplied_token.iter().zip(expected_token).fold(0u8, |diff, (a, b)| diff | (a ^ b)) != 0 {
            return Err("authentication failed".into());
        }
        match fields[1] {
            1 | 6 | 7 => compute_request(stream, fields, cache).map(Response::Bytes),
            8 => raw_heap_request(stream, fields[2] as usize, cache).map(Response::Bytes),
            2 => graphics_request(stream, fields).map(Response::Bytes),
            5 | 10 | 12 | 14 | 16 | 18 | 20 => {
                let mut payload = vec![0; fields[2] as usize];
                stream.read_exact(&mut payload).map_err(|e| e.to_string())?;
                let batch = victory_gpu::wine_transport::decode_batch(&payload)?;
                if fields[1] != 20 && victory_gpu::wine_transport::requires_cube_protocol(&batch) {
                    return Err("cube views require protocol 20 or 21".into());
                }
                if !matches!(fields[1],18|20) && victory_gpu::wine_transport::requires_array_protocol(&batch) {
                    return Err("texture arrays require protocol 18 or later".into());
                }
                let results = victory_gpu::execute_wine_batch(batch)?;
                victory_gpu::wine_transport::encode_results(&results).map(Response::Bytes)
            }
            9 | 11 | 13 | 15 | 17 | 19 | 21 => {
                let batch = victory_gpu::wine_transport::read_stream_batch(stream)?;
                if fields[1] != 21 && victory_gpu::wine_transport::requires_cube_protocol(&batch) {
                    return Err("cube views require protocol 20 or 21".into());
                }
                if !matches!(fields[1],19|21) && victory_gpu::wine_transport::requires_array_protocol(&batch) {
                    return Err("texture arrays require protocol 19 or later".into());
                }
                let results = victory_gpu::execute_wine_batch(batch)?;
                victory_gpu::wine_transport::validate_stream_results(&results)?;
                Ok(Response::Stream(results))
            }
            4 => present_request(stream, fields, presentation_enabled).map(Response::Bytes),
            _ => unreachable!(),
        }
    }

    fn respond(stream: &mut TcpStream, result: Result<Response, String>) -> io::Result<()> {
        if let Ok(Response::Stream(results)) = &result {
            for value in [MAGIC, 0, 0, 0] {
                stream.write_all(&value.to_le_bytes())?;
            }
            return victory_gpu::wine_transport::write_stream_results(stream, results)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e));
        }
        let (status, data, message) = match result {
            Ok(Response::Bytes(data)) => (0u32, data, String::new()),
            Ok(Response::Stream(_)) => unreachable!(),
            Err(mut message) => {
                if message.len() > MAX_MESSAGE {
                    let mut end = MAX_MESSAGE;
                    while !message.is_char_boundary(end) { end -= 1; }
                    message.truncate(end);
                }
                (1, Vec::new(), message)
            }
        };
        for value in [MAGIC, status, data.len() as u32, message.len() as u32] {
            stream.write_all(&value.to_le_bytes())?;
        }
        stream.write_all(&data)?;
        stream.write_all(message.as_bytes())
    }

    pub fn run() -> Result<(), String> {
        let mut args = std::env::args();
        let program = args.next().unwrap_or_default();
        let address: SocketAddr = args.next().ok_or_else(|| format!("usage: {program} LOOPBACK_ADDR:PORT [--announce]"))?
            .parse().map_err(|e| format!("invalid listen address: {e}"))?;
        let announce = match args.next().as_deref() {
            None => false,
            Some("--announce") => true,
            _ => return Err("expected optional --announce after the loopback address".into()),
        };
        if args.next().is_some() || !address.ip().is_loopback() {
            return Err("expected one loopback listen address and optional --announce".into());
        }
        let token = token()?;
        let listener = TcpListener::bind(address).map_err(|e| e.to_string())?;
        let presentation_enabled = std::env::var_os("VICTORY_WINE_METAL_PRESENT")
            .is_some_and(|value| value == "1");
        if presentation_enabled {
            let (send, receive) = std::sync::mpsc::sync_channel(1);
            victory_metal::present::run_app(move || {
                let _ = send.send(serve(listener, token, true, announce));
            }).map_err(|e| e.to_string())?;
            receive.recv().map_err(|e| e.to_string())?
        } else {
            serve(listener, token, false, announce)
        }
    }

    fn announce_listener(listener: &TcpListener, out: &mut impl Write) -> io::Result<()> {
        writeln!(out, "VICTORY_WINE_METAL_READY={}", listener.local_addr()?)?;
        out.flush()
    }

    fn serve(listener: TcpListener, token: [u8; 16], presentation_enabled: bool, announce: bool) -> Result<(), String> {
        if announce {
            announce_listener(&listener, &mut io::stdout().lock()).map_err(|e| e.to_string())?;
        }
        let mut cache = None;
        for connection in listener.incoming() {
            match connection {
                Ok(mut stream) => {
                    if !stream.peer_addr().map(|a| a.ip().is_loopback()).unwrap_or(false) {
                        continue;
                    }
                    if let Err(error) = stream.set_read_timeout(Some(Duration::from_secs(30)))
                        .and_then(|()| stream.set_write_timeout(Some(Duration::from_secs(60))))
                    {
                        eprintln!("setting connection timeouts failed: {error}");
                        continue;
                    }
                    let result = request(&mut stream, &token, &mut cache, presentation_enabled);
                    if let Err(error) = respond(&mut stream, result) {
                        eprintln!("response failed: {error}");
                    }
                }
                Err(error) => eprintln!("accept failed: {error}"),
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn announces_bound_ephemeral_loopback_port() {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let mut announcement = Vec::new();
            announce_listener(&listener, &mut announcement).unwrap();
            let line = std::str::from_utf8(&announcement).unwrap();
            assert_eq!(line, format!("VICTORY_WINE_METAL_READY={}\n", listener.local_addr().unwrap()));
            let endpoint: SocketAddr = line.trim_end().strip_prefix("VICTORY_WINE_METAL_READY=").unwrap().parse().unwrap();
            assert_ne!(endpoint.port(), 0);
            let client = TcpStream::connect(endpoint).unwrap();
            let (server, _) = listener.accept().unwrap();
            drop((client, server));
        }

        #[test]
        fn stream_authentication_reserved_fields_and_reply() {
            for (reserved, token, expected) in [
                (1, [0;16], "reserved"),
                (0, [1;16], "authentication"),
            ] {
                let listener=TcpListener::bind("127.0.0.1:0").unwrap();
                let mut client=TcpStream::connect(listener.local_addr().unwrap()).unwrap();
                for value in [MAGIC,9,reserved,0,0,0,0,0] {
                    client.write_all(&value.to_le_bytes()).unwrap();
                }
                client.write_all(&token).unwrap();
                let (mut server,_)=listener.accept().unwrap();
                let result=request(&mut server,&[0;16],&mut None,false);
                assert!(result.unwrap_err().contains(expected));
            }
            let listener=TcpListener::bind("127.0.0.1:0").unwrap();
            let mut client=TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            let (mut server,_)=listener.accept().unwrap();
            respond(&mut server,Ok(Response::Stream(vec![]))).unwrap();
            let mut header=[0;16];
            client.read_exact(&mut header).unwrap();
            assert_eq!(&header[..4],&MAGIC.to_le_bytes());
            assert_eq!(&header[4..],&[0;12]);
            assert!(victory_gpu::wine_transport::read_stream_results(&mut client).unwrap().is_empty());
        }

        #[test]
        fn extended_batch_versions_validate_headers_and_authentication() {
            for version in [12u32,13,14,15,16,17,18,19,20,21] {
                for (reserved, token, expected) in [
                    (1, [0;16], "reserved"),
                    (0, [1;16], "authentication"),
                ] {
                    let listener=TcpListener::bind("127.0.0.1:0").unwrap();
                    let mut client=TcpStream::connect(listener.local_addr().unwrap()).unwrap();
                    let fields=if matches!(version,12|14|16|18|20) {[MAGIC,version,0,reserved,0,0,0,0]}
                        else {[MAGIC,version,reserved,0,0,0,0,0]};
                    for value in fields {client.write_all(&value.to_le_bytes()).unwrap();}
                    client.write_all(&token).unwrap();
                    let (mut server,_)=listener.accept().unwrap();
                    assert!(request(&mut server,&[0;16],&mut None,false).unwrap_err().contains(expected));
                }
            }
        }

        #[test]
        fn addressed_batch_versions_dispatch_framed_and_streamed_payloads() {
            for version in [14u32,15,16,17] {
                let listener=TcpListener::bind("127.0.0.1:0").unwrap();
                let mut client=TcpStream::connect(listener.local_addr().unwrap()).unwrap();
                let fields=if version==14 || version==16 {[MAGIC,version,4,0,0,0,0,0]}
                    else {[MAGIC,version,0,0,0,0,0,0]};
                for value in fields {client.write_all(&value.to_le_bytes()).unwrap();}
                client.write_all(&[0;16]).unwrap();
                client.write_all(&0u32.to_le_bytes()).unwrap();
                if version==15 || version==17 {client.write_all(&0u32.to_le_bytes()).unwrap();}
                let (mut server,_)=listener.accept().unwrap();
                let error=request(&mut server,&[0;16],&mut None,false).unwrap_err();
                assert!(!error.contains("protocol header") && !error.contains("authentication"),"{error}");
            }
        }

        #[test]
        fn rejects_old_batch_opcode() {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
            for word in [MAGIC, 3, 0, 0, 0, 0, 0, 0] {
                client.write_all(&word.to_le_bytes()).unwrap();
            }
            let (mut server, _) = listener.accept().unwrap();
            let error = request(&mut server, &[0; 16], &mut None, false).unwrap_err();
            assert!(error.contains("invalid protocol header"));
        }
    }
}

#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
fn main() {
    if let Err(error) = host::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

#[cfg(not(all(target_os = "macos", target_arch = "x86_64")))]
fn main() {
    eprintln!("wine_metal_compute_host requires macOS x86_64");
    std::process::exit(1);
}
