//! Restricted vertex/pixel pipelines for the offscreen Metal renderer.
use std::ffi::CStr;
use victory_shader::{Container, FourCC, HwStage, emit, io, ir, native, resource};

pub(crate) struct Prepared {
    vertex: Vec<u8>,
    pixel: Vec<u8>,
    pub(crate) user_data: Vec<emit::UserData>,
    pub(crate) dwords: usize,
    renderer: Option<(u32, u32, victory_metal::Renderer)>,
}

// The renderer owns Metal objects and is only accessed through the pipeline's
// mutex; its synchronous draws complete before another thread can use it.
unsafe impl Send for Prepared {}

impl Prepared {
    pub(crate) fn new(vertex: &[u8], pixel: &[u8]) -> Result<Self, String> {
        let (vertex, _, _) = convert(vertex, HwStage::Vs)?;
        let (pixel, user_data, dwords) = convert(pixel, HwStage::Ps)?;
        Ok(Self {
            vertex,
            pixel,
            user_data,
            dwords,
            renderer: None,
        })
    }

    pub(crate) fn draw(
        &mut self,
        width: u32,
        height: u32,
        vertices: &[[f32; 4]],
        constants: &[u32],
        pixels: &mut [u8],
    ) -> Result<(), String> {
        if !matches!(&self.renderer, Some((w, h, _)) if *w == width && *h == height) {
            let renderer = victory_metal::Renderer::new(
                &self.vertex,
                "main",
                &self.pixel,
                "main",
                width,
                height,
                self.dwords as u32,
                false,
            )
            .map_err(|e| e.to_string())?;
            self.renderer = Some((width, height, renderer));
        }
        self.renderer
            .as_mut()
            .unwrap()
            .2
            .draw_preserving(vertices, constants, pixels)
            .map_err(|e| e.to_string())
    }
}

fn word(data: &[u8], at: usize) -> Result<u32, String> {
    let bytes = data
        .get(at..at.checked_add(4).ok_or("offset overflow")?)
        .ok_or("truncated metadata")?;
    Ok(u32::from_le_bytes(bytes.try_into().unwrap()))
}

// Signature records are the standard 32-byte ISG1/OSG1 layout. Requiring a
// single full register prevents silently dropping interpolants or outputs.
fn signature(
    container: &Container<'_>,
    part: FourCC,
    expected: Option<(&[u8], u32)>,
) -> Result<(), String> {
    let Some(data) = container.part(part).map(|p| p.data) else {
        return if expected.is_none() {
            Ok(())
        } else {
            Err(format!("missing {part} signature"))
        };
    };
    let count = word(data, 0)?;
    if expected.is_none() && count == 0 {
        return Ok(());
    }
    let (name, system) = expected.ok_or_else(|| format!("unexpected {part} signature"))?;
    if count != 1 {
        return Err(format!("{part} must have exactly one element"));
    }
    let at = word(data, 4)? as usize;
    let name_at = word(data, at + 4)? as usize;
    let bytes = data.get(name_at..).ok_or("signature name out of bounds")?;
    let end = bytes
        .iter()
        .position(|&b| b == 0)
        .ok_or("unterminated signature name")?;
    if !bytes[..end].eq_ignore_ascii_case(name)
        || word(data, at)? != 0
        || word(data, at + 8)? != 0
        || word(data, at + 12)? != system
        || word(data, at + 16)? != 3
        || word(data, at + 20)? != 0
        || data.get(at + 24) != Some(&15)
        || word(data, at + 28)? != 0
    {
        return Err(format!("unsupported {part} signature"));
    }
    Ok(())
}

fn convert(bytes: &[u8], stage: HwStage) -> Result<(Vec<u8>, Vec<emit::UserData>, usize), String> {
    let c = Container::parse(bytes).map_err(|e| e.to_string())?;
    let dxil = c.part(FourCC::DXIL).ok_or("missing DXIL part")?;
    let programs = victory_shader::native_programs(dxil.data).map_err(|e| e.to_string())?;
    if programs.len() != 1 || programs[0].stage != stage {
        return Err("expected exactly one native program of the requested stage".into());
    }
    let p = &programs[0];
    if !p.used.is_empty()
        || (stage == HwStage::Vs && !p.layout.is_empty())
        || p.layout
            .iter()
            .any(|r| r.root != native::RootType::Constants || r.slot >= 64)
        || p.layout
            .first()
            .is_some_and(|first| p.layout.iter().any(|r| r.slot != first.slot))
    {
        return Err("only one pixel root-constants parameter is supported".into());
    }
    match (&p.io, stage) {
        (native::StageIo::Vs { inputs, outputs }, HwStage::Vs)
            if inputs.len() == 1
                && inputs[0].usage == native::Usage::Generic
                && inputs[0].row == 0
                && inputs[0].mask == 15
                && outputs.is_empty() =>
        {
            signature(&c, FourCC::ISG1, Some((b"POSITION", 0)))?;
            signature(&c, FourCC::OSG1, Some((b"SV_Position", 1)))?;
        }
        (native::StageIo::Ps { inputs, .. }, HwStage::Ps) if inputs.is_empty() => {
            // Position may be declared without being used by the pixel shader.
            let ps_input = c.part(FourCC::ISG1).map(|p| p.data);
            if ps_input.map(|d| word(d, 0)).transpose()?.unwrap_or(0) == 0 {
                signature(&c, FourCC::ISG1, None)?;
            } else {
                signature(&c, FourCC::ISG1, Some((b"SV_Position", 1)))?;
            }
            signature(&c, FourCC::OSG1, Some((b"SV_Target", 64)))?;
        }
        _ => return Err("unsupported native stage inputs or outputs".into()),
    }
    let program = ir::build(&p.patched_code(), p.wave64, p.vgprs).map_err(|e| e.to_string())?;
    let traced = resource::track(&program, &p.layout, p.stage).map_err(|e| e.to_string())?;
    if !traced.scalar.is_empty() || !traced.descriptors.is_empty() {
        return Err("shader accesses resources".into());
    }
    let raised = io::raise(&program, p).map_err(|e| e.to_string())?;
    if raised.inputs.iter().any(|(_, input)| match stage {
        HwStage::Vs => !matches!(input, io::Input::Attribute { row: 0, .. }),
        HwStage::Ps => !matches!(input, io::Input::Position(_)),
        _ => true,
    }) {
        return Err("unsupported shader system inputs".into());
    }
    let valid_target = |t| match stage {
        HwStage::Vs => matches!(
            t,
            io::Target::Position(0) | io::Target::Unread | io::Target::Null
        ),
        HwStage::Ps => matches!(t, io::Target::Color(0) | io::Target::Null),
        _ => false,
    };
    if !raised.interps.is_empty()
        || raised.exports.iter().any(|e| !valid_target(e.target))
        || !raised.exports.iter().any(|e| match stage {
            HwStage::Vs => matches!(e.target, io::Target::Position(0)),
            _ => matches!(e.target, io::Target::Color(0)),
        })
    {
        return Err("unsupported shader exports or interpolation".into());
    }
    let host = emit::translate(bytes).map_err(|e| e.to_string())?;
    if host.stage != stage
        || (stage == HwStage::Vs && !host.user_data.is_empty())
        || host
            .user_data
            .iter()
            .any(|u| u.root != native::RootType::Constants || u.slot >= 64 || u.dword >= 64)
        || host
            .user_data
            .first()
            .is_some_and(|first| host.user_data.iter().any(|u| u.slot != first.slot))
        || !host.samplers.is_empty()
        || !host.static_samplers.is_empty()
        || !host.root_cbvs.is_empty()
        || !host.root_srvs.is_empty()
    {
        return Err("translated shader has unsupported bindings".into());
    }
    let path = std::env::var_os("VICTORY_METAL_CONVERTER").ok_or("set VICTORY_METAL_CONVERTER")?;
    let dwords = host.user_data_dwords() as usize;
    if dwords > 64 {
        return Err("fragment user-data buffer is too large".into());
    }
    let metallib = if stage == HwStage::Vs {
        victory_metal::converter::convert(
            std::path::Path::new(&path),
            &host.dxil,
            victory_metal::converter::Stage::Vertex,
        )?
    } else {
        victory_metal::converter::convert_fragment_with_b0(
            std::path::Path::new(&path),
            &host.dxil,
            dwords != 0,
        )?
    };
    Ok((metallib, host.user_data, dwords))
}

#[derive(Default)]
pub(crate) struct StreamState {
    seen: u32,
    error: Option<String>,
}

impl StreamState {
    pub(crate) fn reject(&mut self, reason: &str) {
        if self.error.is_none() {
            self.error = Some(reason.into());
        }
    }

    /// `at` points to a complete subobject and any referenced input layout is
    /// valid for the duration of pipeline creation, as required by D3D12.
    pub(crate) unsafe fn read(&mut self, kind: u32, at: *const u8, size: usize) {
        if self.seen & (1 << kind) != 0 {
            self.reject("duplicate pipeline subobject");
            return;
        }
        self.seen |= 1 << kind;
        let data = unsafe { std::slice::from_raw_parts(at, size) };
        let get = |offset| word(data, offset).unwrap();
        let valid = match kind {
            0 => data[8..16] == [0; 8],
            1 | 2 => true,
            // Solid, no culling, no bias, depth clipping on, no MSAA or conservative raster.
            10 => {
                get(4) == 3
                    && get(8) == 1
                    && get(16) == 0
                    && get(20) == 0
                    && get(24) == 0
                    && get(28) == 1
                    && get(32) == 0
                    && get(36) == 0
                    && get(40) == 0
                    && get(44) == 0
            }
            // Depth and stencil disabled; other fields are inactive.
            11 => get(4) == 0 && get(16) == 0,
            12 => unsafe { input_layout(data) },
            13 => get(4) == 0,
            14 => get(4) == 3,
            15 => get(4) == 28 && (1..8).all(|i| get(4 + i * 4) == 0) && get(36) == 1,
            16 => get(4) == 0,
            17 => get(4) == 1 && get(8) == 0,
            9 => get(4) == u32::MAX,
            _ => false,
        };
        if !valid {
            self.reject(&format!("unsupported graphics stream subobject {kind}"));
        }
    }

    pub(crate) fn finish(self) -> Result<(), String> {
        if let Some(error) = self.error {
            return Err(error);
        }
        let required = [1, 2, 10, 11, 12, 14, 15]
            .into_iter()
            .fold(0, |m, k| m | (1 << k));
        if self.seen & required != required {
            return Err("graphics stream lacks required shader, rasterizer, depth, input, topology or target state".into());
        }
        Ok(())
    }
}

unsafe fn input_layout(data: &[u8]) -> bool {
    let ptr = u64::from_le_bytes(data[8..16].try_into().unwrap()) as *const u8;
    if ptr.is_null() || word(data, 16).ok() != Some(1) {
        return false;
    }
    // D3D12_INPUT_ELEMENT_DESC: name pointer followed by six u32 fields.
    let name = unsafe { ptr.cast::<*const i8>().read_unaligned() };
    if name.is_null()
        || unsafe { CStr::from_ptr(name) }
            .to_bytes()
            .eq_ignore_ascii_case(b"POSITION")
            == false
    {
        return false;
    }
    let element = unsafe { std::slice::from_raw_parts(ptr, 32) };
    [0, 2, 0, 0, 0, 0]
        .iter()
        .enumerate()
        .all(|(i, expected)| word(element, 8 + i * 4).ok() == Some(*expected))
}
