//! Synchronous execution of copied Wine graphics batches on the Metal renderer.
use std::collections::HashMap;
use crate::metal_graphics::{GraphicsState, Prepared, Texture, Vertices};
use crate::wine_transport::{Batch, Command, CopyTexture, Draw, Pipeline, Resource, ResourceKind};
use victory_metal::{BlendState, ColorFormat, DepthState, IndexData, TextureFilter, TextureFormat};
use victory_shader::native::RootType;

fn format(n:u32)->Result<ColorFormat,String> { match n {28=>Ok(ColorFormat::Rgba8),29=>Ok(ColorFormat::Rgba8Srgb),87=>Ok(ColorFormat::Bgra8),91=>Ok(ColorFormat::Bgra8Srgb),_=>Err("unsupported texture format".into())} }
fn image(r:&Resource)->Result<(u32,u32,ColorFormat),String> {
    let ResourceKind::Texture {width,height,format:fmt}=r.kind else {return Err("expected texture".into())};
    let size=(width as usize).checked_mul(height as usize).and_then(|n|n.checked_mul(4)).ok_or("texture size overflow")?;
    if width==0 || height==0 || size!=r.bytes.len() {return Err("invalid texture dimensions or byte length".into())} Ok((width,height,format(fmt)?))
}
fn range(offset:u64,size:u64,len:usize)->Result<std::ops::Range<usize>,String> {
    let start=usize::try_from(offset).map_err(|_|"offset overflow")?;
    let end=start.checked_add(usize::try_from(size).map_err(|_|"size overflow")?).ok_or("range overflow")?;
    if end>len {return Err("resource range out of bounds".into())} Ok(start..end)
}
fn buffer(r:&Resource)->Result<(),String> {if !matches!(r.kind,ResourceKind::Buffer) {Err("expected buffer".into())} else {Ok(())}}
fn supports_state(texture:bool,state:u32)->bool {
    if texture {matches!(state,0|4|0x10|0x80|0x400|0x800)} else {matches!(state,0|1|2|3|8|0x400|0x800)}
}
fn promotable(texture:bool,state:u32)->bool {!texture || matches!(state,0x80|0x400|0x800)}
fn access(rs:&mut HashMap<u64,Resource>,touched:&mut HashMap<u64,bool>,id:u64,required:u32)->Result<(),String> {
    let r=rs.get_mut(&id).ok_or("unknown resource")?;let texture=matches!(r.kind,ResourceKind::Texture {..});
    if !supports_state(texture,required) {return Err("unsupported resource state".into())}
    let allowed=r.state==required || (matches!(required,1|2) && r.state==3);
    if !allowed && !(r.state==0 && promotable(texture,required)) {return Err("resource state does not permit command".into())}
    touched.entry(id).or_insert(!texture || r.state==0);
    if r.state==0 {r.state=required;} Ok(())
}
fn copy_texture(rs:&mut HashMap<u64,Resource>,c:CopyTexture)->Result<(),String> {
    if c.src==c.dst {return Err("texture copy aliases itself".into())}
    let (tid,bid)=if c.buffer_to_texture {(c.dst,c.src)} else {(c.src,c.dst)};
    let texture=rs.get(&tid).ok_or("unknown texture")?;
    let ResourceKind::Texture {width,height,format:fmt}=texture.kind else {return Err("expected texture".into())};
    if fmt==40 {depth_image(texture)?;} else {image(texture)?;}
    if width!=c.width || height!=c.height || c.format!=fmt {return Err("copy texture description mismatch".into())}
    let row=(width as usize).checked_mul(4).ok_or("row overflow")?;
    let pitch=c.pitch as usize; if pitch<row {return Err("texture copy pitch too small".into())}
    let buf=rs.get(&bid).ok_or("unknown buffer")?; buffer(buf)?;
    let span=(height as usize-1).checked_mul(pitch).and_then(|n|n.checked_add(row)).ok_or("copy size overflow")?;
    let start=range(c.offset,span as u64,buf.bytes.len())?.start;
    let mut dst=rs.remove(&c.dst).ok_or("unknown destination")?;
    let src=rs.get(&c.src).ok_or("unknown source")?;
    for y in 0..height as usize { let a=y*row; let b=start+y*pitch; if c.buffer_to_texture {dst.bytes[a..a+row].copy_from_slice(&src.bytes[b..b+row]);} else {dst.bytes[b..b+row].copy_from_slice(&src.bytes[a..a+row]);} }
    rs.insert(c.dst,dst); Ok(())
}
enum Indices { U16(Vec<u16>), U32(Vec<u32>) }
fn depth_image(r:&Resource)->Result<(u32,u32),String> {
    let ResourceKind::Texture {width,height,format:40}=r.kind else {return Err("expected D32_FLOAT texture".into())};
    let size=(width as usize).checked_mul(height as usize).and_then(|n|n.checked_mul(4)).ok_or("depth size overflow")?;
    if width==0 || height==0 || r.bytes.len()!=size {return Err("invalid depth texture size".into())} Ok((width,height))
}
fn draw(rs:&mut HashMap<u64,Resource>,pipelines:&HashMap<u64,Pipeline>,prepared:&mut HashMap<u64,Prepared>,d:Draw)->Result<(),String> {
    let p=pipelines.get(&d.pipeline).ok_or("unknown pipeline")?;
    if d.instance_count!=1 || d.first_instance!=0 || d.topology!=4 || !matches!(d.vertex_stride,16|24) || d.vertex_stride!=p.vertex_stride || d.vertex_count==0 || d.vertex_count%3!=0 {return Err("unsupported draw topology, instances or vertex layout".into())}
    if d.target==d.vertex_buffer || d.texture==Some(d.target) || d.depth==Some(d.target) || d.depth==d.texture && d.depth.is_some() {return Err("draw resource alias is unsupported".into())}
    let target=rs.get(&d.target).ok_or("unknown target")?; let (width,height,color_format)=image(target)?;
    if format(p.format)?!=color_format || p.depth.is_some()!=d.depth.is_some() {return Err("pipeline attachment mismatch".into())}
    let vb=rs.get(&d.vertex_buffer).ok_or("unknown vertex buffer")?; buffer(vb)?;
    let view=range(d.vertex_offset,d.vertex_size as u64,vb.bytes.len())?;
    let (first,size)=if d.index.is_some() {(0,view.len()/d.vertex_stride as usize*d.vertex_stride as usize)} else {((d.first_vertex as usize).checked_mul(d.vertex_stride as usize).ok_or("vertex offset overflow")?,(d.vertex_count as usize).checked_mul(d.vertex_stride as usize).ok_or("vertex size overflow")?)};
    let end=first.checked_add(size).ok_or("vertex range overflow")?; if end>view.len() {return Err("vertex view out of bounds".into())}
    let words:Vec<f32>=vb.bytes[view.start+first..view.start+end].chunks_exact(4).map(|b|f32::from_bits(u32::from_le_bytes(b.try_into().unwrap()))).collect();
    let indices=if let Some(i)=&d.index {
        if d.first_vertex!=0 {return Err("indexed draw first vertex must be zero".into())}
        let ib=rs.get(&i.buffer).ok_or("unknown index buffer")?;buffer(ib)?;let view=range(i.offset,i.size as u64,ib.bytes.len())?;
        let element=match i.format {57=>2usize,42=>4,_=>return Err("unsupported index format".into())};
        let start=(i.first as usize).checked_mul(element).ok_or("index offset overflow")?;let len=(d.vertex_count as usize).checked_mul(element).ok_or("index size overflow")?;
        let end=start.checked_add(len).ok_or("index overflow")?;if end>view.len() {return Err("index view out of bounds".into())}
        let bytes=&ib.bytes[view.start+start..view.start+end];
        let check=|v:u32| {let n=i64::from(v)+i64::from(i.base_vertex);n>=0 && (n as usize)<size/d.vertex_stride as usize};
        if element==2 {let v:Vec<u16>=bytes.chunks_exact(2).map(|b|u16::from_le_bytes(b.try_into().unwrap())).collect();if v.iter().any(|&x|!check(x as u32)) {return Err("index out of bounds".into())} Some(Indices::U16(v))}
        else {let v:Vec<u32>=bytes.chunks_exact(4).map(|b|u32::from_le_bytes(b.try_into().unwrap())).collect();if v.iter().any(|&x|!check(x)) {return Err("index out of bounds".into())} Some(Indices::U32(v))}
    } else {None};
    if !prepared.contains_key(&d.pipeline) {
        let depth=if let Some(z)=&p.depth {if !(1..=8).contains(&z.compare) {return Err("unsupported depth comparison".into())} Some(DepthState {write:z.write,compare:z.compare})} else {None};
        if p.blend.write_mask>15 {return Err("invalid blend mask".into())}
        let state=GraphicsState {depth,blend:BlendState {enabled:p.blend.enabled,write_mask:p.blend.write_mask,alpha_source:p.blend.alpha_source},input_uv:p.vertex_stride==24,color_format};
        prepared.insert(d.pipeline,Prepared::new(&p.vertex_shader,&p.pixel_shader,state)?);
    }
    let prepared=prepared.get_mut(&d.pipeline).unwrap();
    if d.texture_slot!=prepared.texture_slot.map(u32::from) || d.sampler_slot!=prepared.sampler_slot.map(u32::from) || d.texture.is_some()!=prepared.texture_slot.is_some() {return Err("shader texture or sampler slot mismatch".into())}
    let mut constants=vec![0;prepared.dwords];let mut has_constants=false;
    for u in &prepared.user_data {constants[u.index as usize]=match u.root {RootType::Constants=>{has_constants=true;if d.constant_slot!=Some(u.slot as u32) {return Err("constant root slot mismatch".into())} *d.constants.get(u.dword as usize).ok_or("missing root constant")?},RootType::Table=>0,_=>return Err("unsupported root argument".into())};}
    if !has_constants && (d.constant_slot.is_some() || !d.constants.is_empty()) {return Err("unexpected root constants".into())}
    if d.viewport.iter().any(|v|!v.is_finite()) {return Err("invalid viewport".into())}
    let sc=d.scissor;if sc[2]<sc[0] || sc[3]<sc[1] {return Err("invalid scissor".into())}
    let clip=|v:i32,limit:u32| i64::from(v).clamp(0,i64::from(limit)) as u32;
    let (left,top,right,bottom)=(clip(sc[0],width),clip(sc[1],height),clip(sc[2],width),clip(sc[3],height));
    let viewport=d.viewport.map(f64::from); let scissor=[left,top,right-left,bottom-top];
    let mut depth=if let Some(id)=d.depth {let r=rs.get(&id).ok_or("unknown depth target")?;if depth_image(r)?!=(width,height) {return Err("depth dimensions mismatch".into())} Some(r.bytes.chunks_exact(4).map(|b|f32::from_bits(u32::from_le_bytes(b.try_into().unwrap()))).collect::<Vec<_>>())} else {None};
    let mut target=rs.remove(&d.target).ok_or("unknown target")?;
    let sampled=if let Some(id)=d.texture {let r=rs.get(&id).ok_or("unknown sampled texture")?;let (w,h,f)=image(r)?;Some(Texture {width:w,height:h,levels:1,pixels:&r.bytes,filter:if d.linear_filter {TextureFilter::Linear} else {TextureFilter::Nearest},format:TextureFormat::from(f),min_lod:0.,max_lod:0.})} else {None};
    let index=indices.as_ref().map(|v| (match v {Indices::U16(v)=>IndexData::U16(v),Indices::U32(v)=>IndexData::U32(v)},d.index.as_ref().unwrap().base_vertex));
    let result=if d.vertex_stride==16 {let v:Vec<[f32;4]>=words.chunks_exact(4).map(|x|x.try_into().unwrap()).collect();prepared.draw(width,height,Vertices::Position(&v),index,viewport,scissor,depth.as_deref_mut(),&constants,sampled,&mut target.bytes)} else {let v:Vec<[f32;6]>=words.chunks_exact(6).map(|x|x.try_into().unwrap()).collect();prepared.draw(width,height,Vertices::PositionUv(&v),index,viewport,scissor,depth.as_deref_mut(),&constants,sampled,&mut target.bytes)};
    rs.insert(d.target,target);result?;
    if let (Some(id),Some(depth))=(d.depth,depth) {let r=rs.get_mut(&id).ok_or("unknown depth target")?;for (bytes,value) in r.bytes.chunks_exact_mut(4).zip(depth) {bytes.copy_from_slice(&value.to_bits().to_le_bytes());}}
    Ok(())
}
/// Execute a complete batch in order. Returned snapshots are the only committed state.
pub fn execute_wine_batch(batch:Batch)->Result<Vec<Resource>,String> {
    let mut rs=HashMap::new(); for r in batch.resources {if r.id==0 || rs.insert(r.id,r).is_some() {return Err("duplicate resource id".into())}}
    let mut pipelines=HashMap::new();for p in batch.pipelines {if p.id==0 || pipelines.insert(p.id,p).is_some() {return Err("duplicate pipeline id".into())}}
    let mut prepared=HashMap::new();let mut touched=HashMap::new();
    for c in batch.commands {match c {
        Command::Barrier {id,before,after}=>{
            let r=rs.get_mut(&id).ok_or("unknown barrier resource")?;let texture=matches!(r.kind,ResourceKind::Texture {..});
            if !supports_state(texture,before) || !supports_state(texture,after) || (r.state!=before && !(r.state==0 && promotable(texture,before))) {return Err("resource barrier state mismatch or unsupported state".into())}
            let decay=touched.entry(id).or_insert(!texture);if texture {*decay=false;} r.state=after;
        },
        Command::Clear {target,color}=>{access(&mut rs,&mut touched,target,4)?;let r=rs.get_mut(&target).ok_or("unknown clear target")?;let (w,h,f)=image(r)?; victory_metal::clear_color_float(f,w,h,w as usize*4,&mut r.bytes,color,&[]).map_err(|e|e.to_string())?;},
        Command::Draw(d)=>{
            access(&mut rs,&mut touched,d.target,4)?;access(&mut rs,&mut touched,d.vertex_buffer,1)?;
            if let Some(i)=&d.index {access(&mut rs,&mut touched,i.buffer,2)?;}
            if let Some(id)=d.texture {access(&mut rs,&mut touched,id,0x80)?;}
            if let Some(id)=d.depth {access(&mut rs,&mut touched,id,0x10)?;}
            draw(&mut rs,&pipelines,&mut prepared,d)?;
        },
        Command::CopyTexture(c)=>{access(&mut rs,&mut touched,c.src,0x800)?;access(&mut rs,&mut touched,c.dst,0x400)?;copy_texture(&mut rs,c)?;},
        Command::ClearDepth {target,depth}=>{access(&mut rs,&mut touched,target,0x10)?;if !depth.is_finite() || !(0.0..=1.0).contains(&depth) {return Err("invalid depth clear".into())} let r=rs.get_mut(&target).ok_or("unknown depth target")?;depth_image(r)?;for bytes in r.bytes.chunks_exact_mut(4) {bytes.copy_from_slice(&depth.to_bits().to_le_bytes());}},
        Command::CopyBuffer {src,dst,src_offset,dst_offset,size}=>{access(&mut rs,&mut touched,src,0x800)?;access(&mut rs,&mut touched,dst,0x400)?;let s=rs.get(&src).ok_or("unknown source")?;buffer(s)?;let a=range(src_offset,size,s.bytes.len())?;let d=rs.get(&dst).ok_or("unknown destination")?;buffer(d)?;let b=range(dst_offset,size,d.bytes.len())?;if src==dst {rs.get_mut(&dst).unwrap().bytes.copy_within(a,b.start);} else {let mut d=rs.remove(&dst).unwrap();d.bytes[b].copy_from_slice(&rs.get(&src).unwrap().bytes[a]);rs.insert(dst,d);}},
    }}
    for (id,decay) in touched {if decay {rs.get_mut(&id).unwrap().state=0;}}
    let mut out:Vec<_>=rs.into_values().collect();out.sort_unstable_by_key(|r|r.id);Ok(out)
}
