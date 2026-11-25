//! Synchronous execution of copied Wine graphics batches on the Metal renderer.
use std::collections::HashMap;
use crate::metal_graphics::{GraphicsState, Prepared, ResolvedVertexInput, SampledTextureIdentity, Texture, Vertices};
use crate::wine_transport::{block_geometry, mip_layout, texture_size, Batch, Command, CopyTexture, Draw, Pipeline, Resource, ResourceKind};
use victory_metal::{BlendState, ColorFormat, DepthState, IndexData, TextureFilter, TextureFormat};
use victory_shader::native::RootType;

fn format(n:u32)->Result<ColorFormat,String> { match n {28=>Ok(ColorFormat::Rgba8),29=>Ok(ColorFormat::Rgba8Srgb),87=>Ok(ColorFormat::Bgra8),91=>Ok(ColorFormat::Bgra8Srgb),_=>Err("unsupported texture format".into())} }
fn sampled_format(n:u32)->Result<TextureFormat,String> { match n {71=>Ok(TextureFormat::Bc1),72=>Ok(TextureFormat::Bc1Srgb),77=>Ok(TextureFormat::Bc3),78=>Ok(TextureFormat::Bc3Srgb),_=>Ok(TextureFormat::from(format(n)?))} }
fn image(r:&Resource)->Result<(u32,u32,u32,TextureFormat),String> {
    let ResourceKind::Texture {width,height,format:fmt,mips}=r.kind else {return Err("expected texture".into())};
    let size=texture_size(fmt,width,height,mips).ok_or("invalid texture dimensions or mip count")?;
    if size!=r.bytes.len() {return Err("invalid texture byte length".into())} Ok((width,height,mips,sampled_format(fmt)?))
}
fn target_image(r:&Resource)->Result<(u32,u32,ColorFormat),String> {
    let (width,height,mips,_)=image(r)?;
    if mips!=1 {return Err("render target must have one mip".into())}
    let ResourceKind::Texture {format:fmt,..}=r.kind else {return Err("expected texture".into())};
    Ok((width,height,format(fmt)?))
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
    let ResourceKind::Texture {width,height,format:fmt,mips}=texture.kind else {return Err("expected texture".into())};
    if fmt==40 {depth_image(texture)?;} else {image(texture)?;}
    let (mip_width,mip_height,mip_offset,_)=mip_layout(fmt,width,height,mips,c.mip).ok_or("invalid texture mip")?;
    if mip_width!=c.width || mip_height!=c.height || c.format!=fmt {return Err("copy texture description mismatch".into())}
    let (row,rows)=block_geometry(fmt,mip_width,mip_height).ok_or("invalid texture block geometry")?;
    let pitch=c.pitch as usize; if pitch<row || pitch%256!=0 || c.offset%512!=0 {return Err("invalid texture copy pitch or offset".into())}
    let buf=rs.get(&bid).ok_or("unknown buffer")?; buffer(buf)?;
    let span=(rows-1).checked_mul(pitch).and_then(|n|n.checked_add(row)).ok_or("copy size overflow")?;
    let start=range(c.offset,u64::try_from(span).map_err(|_|"copy size overflow")?,buf.bytes.len())?.start;
    let mut dst=rs.remove(&c.dst).ok_or("unknown destination")?;
    let src=rs.get(&c.src).ok_or("unknown source")?;
    for y in 0..rows { let a=mip_offset+y*row; let b=start+y*pitch; if c.buffer_to_texture {dst.bytes[a..a+row].copy_from_slice(&src.bytes[b..b+row]);} else {dst.bytes[b..b+row].copy_from_slice(&src.bytes[a..a+row]);} }
    rs.insert(c.dst,dst); Ok(())
}
enum Indices { U16(Vec<u16>), U32(Vec<u32>) }
fn depth_image(r:&Resource)->Result<(u32,u32),String> {
    let ResourceKind::Texture {width,height,format:fmt @ (20|40),mips:1}=r.kind else {return Err("expected single-mip D32_FLOAT or D32_FLOAT_S8X24 texture".into())};
    let size=texture_size(fmt,width,height,1).ok_or("invalid depth texture dimensions")?;
    if r.bytes.len()!=size {return Err("invalid depth texture size".into())} Ok((width,height))
}
fn draw(rs:&mut HashMap<u64,Resource>,touched:&mut HashMap<u64,bool>,pipelines:&HashMap<u64,Pipeline>,prepared:&mut HashMap<u64,(Prepared,Vec<ResolvedVertexInput>)>,generations:&HashMap<u64,usize>,d:Draw,stencil_reference:u32)->Result<(),String> {
    let p=pipelines.get(&d.pipeline).ok_or("unknown pipeline")?;
    let generic=!p.inputs.is_empty();
    if d.instance_count==0 || d.topology!=4 || d.vertex_count==0 || d.vertex_count%3!=0
        || (!generic && (d.instance_count!=1 || d.first_instance!=0 || !d.vertices.is_empty() || !matches!(d.vertex_stride,16|24) || d.vertex_stride!=p.vertex_stride))
    {return Err("unsupported draw topology, instances or vertex layout".into())}
    if p.extra_targets.len()>7 || d.extra_targets.len()!=p.extra_targets.len() {
        return Err("pipeline color attachment count mismatch".into());
    }
    let target_ids:Vec<u64>=std::iter::once(d.target).chain(d.extra_targets.iter().copied()).collect();
    if target_ids.iter().enumerate().any(|(i,id)| *id==0 || target_ids[..i].contains(id)
        || *id==d.vertex_buffer || Some(*id)==d.depth
        || d.vertices.iter().any(|v|v.buffer==*id)
        || d.index.as_ref().is_some_and(|v|v.buffer==*id))
    {return Err("draw resource alias is unsupported".into())}
    let target=rs.get(&d.target).ok_or("unknown target")?; let (width,height,color_format)=target_image(target)?;
    if format(p.format)?!=color_format || p.depth.is_some()!=d.depth.is_some() {return Err("pipeline attachment mismatch".into())}
    for (id,desc) in d.extra_targets.iter().zip(&p.extra_targets) {
        let r=rs.get(id).ok_or("unknown target")?;
        let (w,h,f)=target_image(r)?;
        if (w,h)!=(width,height) || f!=format(desc.format)? {return Err("pipeline attachment mismatch".into())}
    }
    let vb=rs.get(&d.vertex_buffer).ok_or("unknown vertex buffer")?; buffer(vb)?;
    let view=range(d.vertex_offset,d.vertex_size as u64,vb.bytes.len())?;
    let (first,size)=if generic {(0,0)} else if d.index.is_some() {(0,view.len()/d.vertex_stride as usize*d.vertex_stride as usize)} else {((d.first_vertex as usize).checked_mul(d.vertex_stride as usize).ok_or("vertex offset overflow")?,(d.vertex_count as usize).checked_mul(d.vertex_stride as usize).ok_or("vertex size overflow")?)};
    let end=first.checked_add(size).ok_or("vertex range overflow")?; if end>view.len() {return Err("vertex view out of bounds".into())}
    let words:Vec<f32>=vb.bytes[view.start+first..view.start+end].chunks_exact(4).map(|b|f32::from_bits(u32::from_le_bytes(b.try_into().unwrap()))).collect();
    let indices=if let Some(i)=&d.index {
        if d.first_vertex!=0 {return Err("indexed draw first vertex must be zero".into())}
        let ib=rs.get(&i.buffer).ok_or("unknown index buffer")?;buffer(ib)?;let view=range(i.offset,i.size as u64,ib.bytes.len())?;
        let element=match i.format {57=>2usize,42=>4,_=>return Err("unsupported index format".into())};
        let start=(i.first as usize).checked_mul(element).ok_or("index offset overflow")?;let len=(d.vertex_count as usize).checked_mul(element).ok_or("index size overflow")?;
        let end=start.checked_add(len).ok_or("index overflow")?;if end>view.len() {return Err("index view out of bounds".into())}
        let bytes=&ib.bytes[view.start+start..view.start+end];
        let check=|v:u32| {let n=i64::from(v)+i64::from(i.base_vertex);n>=0 && (generic || (n as usize)<size/d.vertex_stride as usize)};
        if element==2 {let v:Vec<u16>=bytes.chunks_exact(2).map(|b|u16::from_le_bytes(b.try_into().unwrap())).collect();if v.iter().any(|&x|!check(x as u32)) {return Err("index out of bounds".into())} Some(Indices::U16(v))}
        else {let v:Vec<u32>=bytes.chunks_exact(4).map(|b|u32::from_le_bytes(b.try_into().unwrap())).collect();if v.iter().any(|&x|!check(x)) {return Err("index out of bounds".into())} Some(Indices::U32(v))}
    } else {None};
    if !prepared.contains_key(&d.pipeline) {
        let depth=if let Some(z)=&p.depth {if !(1..=8).contains(&z.compare) {return Err("unsupported depth comparison".into())} Some(DepthState {write:z.write,compare:z.compare,stencil:z.stencil.as_ref().map(|s|victory_metal::StencilState {enabled:s.enabled,read_mask:s.read_mask,write_mask:s.write_mask,front:s.front,back:s.back})})} else {None};
        if p.blend.write_mask>15 {return Err("invalid blend mask".into())}
        let state=GraphicsState {depth,blend:BlendState {enabled:p.blend.enabled,write_mask:p.blend.write_mask,factors:p.blend.factors},input_uv:p.vertex_stride==24,color_format};
        let mut extra=Vec::with_capacity(p.extra_targets.len());
        for target in &p.extra_targets {
            if target.blend.write_mask>15 {return Err("invalid blend mask".into())}
            extra.push((format(target.format)?,BlendState {enabled:target.blend.enabled,write_mask:target.blend.write_mask,factors:target.blend.factors}));
        }
        let value=if generic { Prepared::new_inputs_mrt(&p.vertex_shader,&p.pixel_shader,state,&p.inputs,&extra)? }
            else { (Prepared::new_mrt(&p.vertex_shader,&p.pixel_shader,state,&extra)?,Vec::new()) };
        prepared.insert(d.pipeline,value);
    }
    let (prepared,resolved)=prepared.get_mut(&d.pipeline).unwrap();
    let extended=!d.textures.is_empty() || !d.samplers.is_empty();
    if extended && (d.texture.is_some() || d.texture_slot.is_some() || d.sampler_slot.is_some()
        || d.first_mip!=0 || d.mip_count!=0 || d.linear_filter)
    {return Err("mixed legacy and extended texture bindings".into())}
    let mut texture_bindings:Vec<_>=d.textures.iter().map(|t|(t.slot,t.offset,t.resource,t.first_mip,t.mip_count)).collect();
    let mut sampler_bindings:Vec<_>=d.samplers.iter().map(|s|(s.slot,s.offset,s.linear_filter)).collect();
    if !extended {
        if d.texture.is_none() && d.texture_slot.is_none() && d.sampler_slot.is_none()
            && (!prepared.texture_entries.is_empty() || !prepared.sampler_entries.is_empty())
        {return Err("missing static descriptor binding".into())}
        if d.texture_slot!=prepared.texture_slot.map(u32::from) || d.sampler_slot!=prepared.sampler_slot.map(u32::from)
            || d.texture.is_some()!=prepared.texture_slot.is_some()
            || prepared.texture_slots.len()>1 || prepared.sampler_slots.len()>1
        {return Err("shader texture or sampler slot mismatch".into())}
        if let (Some(slot),Some(resource))=(d.texture_slot,d.texture) {
            texture_bindings.push((slot,0,resource,d.first_mip,d.mip_count));
        } else if d.first_mip!=0 || d.mip_count!=0 {return Err("unexpected texture view range".into())}
        if let Some(slot)=d.sampler_slot {sampler_bindings.push((slot,0,d.linear_filter));}
    }
    texture_bindings.sort_unstable_by_key(|t|(t.0,t.1));
    sampler_bindings.sort_unstable_by_key(|s|(s.0,s.1));
    if texture_bindings.len()>256 || sampler_bindings.len()>256
        || texture_bindings.iter().any(|t|t.0>=64 || t.1>=256 || t.2==0)
        || sampler_bindings.iter().any(|s|s.0>=64 || s.1>=256)
        || texture_bindings.windows(2).any(|w|(w[0].0,w[0].1)==(w[1].0,w[1].1))
        || sampler_bindings.windows(2).any(|w|(w[0].0,w[0].1)==(w[1].0,w[1].1))
        || texture_bindings.iter().any(|t|sampler_bindings.iter().any(|s|s.0==t.0))
    {return Err("invalid descriptor bindings".into())}
    let texture_bindings:Vec<_>=prepared.texture_entries.iter().map(|e|
        texture_bindings.iter().find(|t|t.0==u32::from(e.slot) && t.1==e.offset)
            .copied().ok_or("missing static descriptor binding")
    ).collect::<Result<_,_>>()?;
    let sampler_bindings:Vec<_>=prepared.sampler_entries.iter().map(|e|
        sampler_bindings.iter().find(|s|s.0==u32::from(e.slot) && s.1==e.offset)
            .copied().ok_or("missing static descriptor binding")
    ).collect::<Result<_,_>>()?;
    for &(_,_,id,_,_) in &texture_bindings {
        if target_ids.contains(&id) || Some(id)==d.depth {return Err("draw resource alias is unsupported".into())}
        access(rs,touched,id,0x80)?;
    }
    let mut constants=vec![0;prepared.dwords];let mut has_constants=false;
    for u in &prepared.user_data {constants[u.index as usize]=match u.root {RootType::Constants=>{has_constants=true;if d.constant_slot!=Some(u.slot as u32) {return Err("constant root slot mismatch".into())} *d.constants.get(u.dword as usize).ok_or("missing root constant")?},RootType::Table=>{
        if let Some(entry)=prepared.texture_entries.iter().find(|e|e.slot==u.slot) {(entry.heap_index-entry.offset)*32}
        else if let Some(entry)=prepared.sampler_entries.iter().find(|e|e.slot==u.slot) {(entry.heap_index-entry.offset)*16}
        else {return Err("unknown descriptor table slot".into())}
    },_=>return Err("unsupported root argument".into())};}
    if !has_constants && (d.constant_slot.is_some() || !d.constants.is_empty()) {return Err("unexpected root constants".into())}
    if d.viewport.iter().any(|v|!v.is_finite()) {return Err("invalid viewport".into())}
    let sc=d.scissor;if sc[2]<sc[0] || sc[3]<sc[1] {return Err("invalid scissor".into())}
    let clip=|v:i32,limit:u32| i64::from(v).clamp(0,i64::from(limit)) as u32;
    let (left,top,right,bottom)=(clip(sc[0],width),clip(sc[1],height),clip(sc[2],width),clip(sc[3],height));
    let viewport=d.viewport.map(f64::from); let scissor=[left,top,right-left,bottom-top];
    let mut stencil=None;
    let mut depth=if let Some(id)=d.depth {let r=rs.get(&id).ok_or("unknown depth target")?;if depth_image(r)?!=(width,height) {return Err("depth dimensions mismatch".into())} let combined=matches!(r.kind,ResourceKind::Texture {format:20,..});if combined != p.depth.as_ref().is_some_and(|z|z.stencil.is_some()) {return Err("pipeline depth format mismatch".into())} let size=(width as usize)*(height as usize)*4;if combined {stencil=Some(r.bytes[size..].to_vec());} Some(r.bytes[..size].chunks_exact(4).map(|b|f32::from_bits(u32::from_le_bytes(b.try_into().unwrap()))).collect::<Vec<_>>())} else {None};
    let mut targets:Vec<Vec<u8>>=target_ids.iter().map(|id|std::mem::take(&mut rs.get_mut(id).unwrap().bytes)).collect();
    let mut sampled=Vec::with_capacity(texture_bindings.len());
    for &(_,_,id,first_mip,mip_count) in &texture_bindings {
        let r=rs.get(&id).ok_or("unknown sampled texture")?;let (w,h,mips,f)=image(r)?;
        let end=first_mip.checked_add(mip_count).ok_or("texture view range overflow")?;
        if mip_count==0 || end>mips {return Err("invalid texture view range".into())}
        let ResourceKind::Texture {format:fmt,..}=r.kind else {return Err("expected texture".into())};
        let (view_width,view_height,start,_)=mip_layout(fmt,w,h,mips,first_mip).ok_or("invalid texture view range")?;
        let (_,_,last,last_len)=mip_layout(fmt,w,h,mips,end-1).ok_or("invalid texture view range")?;
        let end_offset=last.checked_add(last_len).ok_or("texture view size overflow")?;
        sampled.push((Texture {width:view_width,height:view_height,levels:mip_count,pixels:&r.bytes[start..end_offset],filter:TextureFilter::Nearest,format:f,min_lod:0.,max_lod:0.},
            Some(SampledTextureIdentity {resource:id,generation:generations.get(&id).copied().unwrap_or(0),first_mip})));
    }
    let samplers:Vec<_>=sampler_bindings.iter().map(|&(_,_,linear)|(if linear {TextureFilter::Linear} else {TextureFilter::Nearest},0.,0.)).collect();
    let index=indices.as_ref().map(|v| (match v {Indices::U16(v)=>IndexData::U16(v),Indices::U32(v)=>IndexData::U32(v)},d.index.as_ref().unwrap().base_vertex));
    let result=if generic {
        let legacy_view=crate::wine_transport::VertexView {slot:0,buffer:d.vertex_buffer,offset:d.vertex_offset,size:d.vertex_size,stride:d.vertex_stride};
        let views=if d.vertices.is_empty() {std::slice::from_ref(&legacy_view)} else {d.vertices.as_slice()};
        let mut buffers=Vec::with_capacity(views.len());
        for v in views {
            let resource=rs.get(&v.buffer).ok_or("unknown vertex buffer")?;buffer(resource)?;
            let bytes=range(v.offset,u64::from(v.size),resource.bytes.len())?;
            buffers.push(victory_metal::VertexBuffer {slot:v.slot,bytes:&resource.bytes[bytes]});
        }
        let mut attributes=Vec::with_capacity(resolved.len());
        for r in resolved.iter() {
            let a=p.inputs.get(r.layout_index).ok_or("invalid reflected vertex input")?;
            let v=views.iter().find(|v|v.slot==a.slot).ok_or("missing vertex buffer")?;
            attributes.push(victory_metal::VertexAttribute {index:u32::from(r.attribute_index),format:a.format,slot:a.slot,offset:a.offset,stride:v.stride,per_instance:a.per_instance,step_rate:a.step_rate});
        }
        let mut outputs:Vec<&mut [u8]>=targets.iter_mut().map(Vec::as_mut_slice).collect();
        prepared.draw_inputs_mrt(width,height,&attributes,&buffers,index,d.first_vertex,d.vertex_count,d.instance_count,d.first_instance,viewport,scissor,depth.as_deref_mut(),stencil.as_deref_mut(),stencil_reference,&constants,&sampled,&samplers,&mut outputs,true)
    } else if !d.extra_targets.is_empty() {
        let bytes=&rs.get(&d.vertex_buffer).ok_or("unknown vertex buffer")?.bytes[view.start+first..view.start+end];
        let buffers=[victory_metal::VertexBuffer {slot:0,bytes}];
        let attributes=prepared.legacy_attributes.clone();
        let mut outputs:Vec<&mut [u8]>=targets.iter_mut().map(Vec::as_mut_slice).collect();
        prepared.draw_inputs_mrt(width,height,&attributes,&buffers,index,0,d.vertex_count,1,0,viewport,scissor,depth.as_deref_mut(),stencil.as_deref_mut(),stencil_reference,&constants,&sampled,&samplers,&mut outputs,true)
    } else if d.vertex_stride==16 {let v:Vec<[f32;4]>=words.chunks_exact(4).map(|x|x.try_into().unwrap()).collect();prepared.draw_multi(width,height,Vertices::Position(&v),index,viewport,scissor,depth.as_deref_mut(),stencil.as_deref_mut(),stencil_reference,&constants,&sampled,&samplers,&mut targets[0])} else {let v:Vec<[f32;6]>=words.chunks_exact(6).map(|x|x.try_into().unwrap()).collect();prepared.draw_multi(width,height,Vertices::PositionUv(&v),index,viewport,scissor,depth.as_deref_mut(),stencil.as_deref_mut(),stencil_reference,&constants,&sampled,&samplers,&mut targets[0])};
    result?;
    for (id,bytes) in target_ids.into_iter().zip(targets) {rs.get_mut(&id).unwrap().bytes=bytes;}
    if let (Some(id),Some(depth))=(d.depth,depth) {let r=rs.get_mut(&id).ok_or("unknown depth target")?;let size=depth.len()*4;for (bytes,value) in r.bytes[..size].chunks_exact_mut(4).zip(depth) {bytes.copy_from_slice(&value.to_bits().to_le_bytes());}if let Some(stencil)=stencil {r.bytes[size..].copy_from_slice(&stencil);}}
    Ok(())
}

// Mark all possible writers before execution. A failing command aborts the
// batch, so its speculative generation can never be observed by a later draw.
fn mark_writes(generations:&mut HashMap<u64,usize>,command:&Command,generation:usize) {
    let (first,second)=match command {
        Command::Clear {target,..} | Command::ClearDepth {target,..} | Command::ClearStencil {target,..} => (Some(*target),None),
        Command::Draw(d) => (Some(d.target),d.depth),
        Command::CopyTexture(c) => (Some(c.dst),None),
        Command::CopyBuffer {dst,..} | Command::CopyDepthStencil {dst,..} => (Some(*dst),None),
        Command::Barrier {..} | Command::SetStencilRef(_) => (None,None),
    };
    for id in [first,second].into_iter().flatten() {generations.insert(id,generation);}
    if let Command::Draw(d)=command {for &id in &d.extra_targets {generations.insert(id,generation);}}
}
/// Execute a complete batch in order. Returned snapshots are the only committed state.
pub fn execute_wine_batch(batch:Batch)->Result<Vec<Resource>,String> {
    let mut rs=HashMap::new(); for r in batch.resources {
        if let ResourceKind::Texture {format:fmt,..}=r.kind {if matches!(fmt,20|40) {depth_image(&r)?;} else {image(&r)?;}}
        if r.id==0 || rs.insert(r.id,r).is_some() {return Err("duplicate resource id".into())}
    }
    let mut pipelines=HashMap::new();for p in batch.pipelines {if p.id==0 || pipelines.insert(p.id,p).is_some() {return Err("duplicate pipeline id".into())}}
    let mut prepared=HashMap::new();let mut touched=HashMap::new();let mut generations=HashMap::new();let mut stencil_reference=0;
    for (index,c) in batch.commands.into_iter().enumerate() {
        mark_writes(&mut generations,&c,index+1);
        match c {
        Command::Barrier {id,before,after}=>{
            let r=rs.get_mut(&id).ok_or("unknown barrier resource")?;let texture=matches!(r.kind,ResourceKind::Texture {..});
            if !supports_state(texture,before) || !supports_state(texture,after) || (r.state!=before && !(r.state==0 && promotable(texture,before))) {return Err("resource barrier state mismatch or unsupported state".into())}
            let decay=touched.entry(id).or_insert(!texture);if texture {*decay=false;} r.state=after;
        },
        Command::Clear {target,color}=>{access(&mut rs,&mut touched,target,4)?;let r=rs.get_mut(&target).ok_or("unknown clear target")?;let (w,h,f)=target_image(r)?; victory_metal::clear_color_float(f,w,h,w as usize*4,&mut r.bytes,color,&[]).map_err(|e|e.to_string())?;},
        Command::Draw(d)=>{
            access(&mut rs,&mut touched,d.target,4)?;
            for &id in &d.extra_targets {access(&mut rs,&mut touched,id,4)?;}
            if d.vertices.is_empty() {access(&mut rs,&mut touched,d.vertex_buffer,1)?;}
            else {for v in &d.vertices {access(&mut rs,&mut touched,v.buffer,1)?;}}
            if let Some(i)=&d.index {access(&mut rs,&mut touched,i.buffer,2)?;}
            // Only statically accessed descriptors participate in this draw.
            if let Some(id)=d.depth {access(&mut rs,&mut touched,id,0x10)?;}
            draw(&mut rs,&mut touched,&pipelines,&mut prepared,&generations,d,stencil_reference)?;
        },
        Command::CopyTexture(c)=>{access(&mut rs,&mut touched,c.src,0x800)?;access(&mut rs,&mut touched,c.dst,0x400)?;copy_texture(&mut rs,c)?;},
        Command::ClearDepth {target,depth}=>{access(&mut rs,&mut touched,target,0x10)?;if !depth.is_finite() || !(0.0..=1.0).contains(&depth) {return Err("invalid depth clear".into())} let r=rs.get_mut(&target).ok_or("unknown depth target")?;depth_image(r)?;if !matches!(r.kind,ResourceKind::Texture {format:40,..}) {return Err("expected D32_FLOAT target".into())}for bytes in r.bytes.chunks_exact_mut(4) {bytes.copy_from_slice(&depth.to_bits().to_le_bytes());}},
        Command::SetStencilRef(value)=>{stencil_reference=value & 255;},
        Command::ClearStencil {target,flags,depth,stencil}=>{
            access(&mut rs,&mut touched,target,0x10)?;
            if flags==0 || flags & !3 != 0 || (flags & 1 != 0 && (!depth.is_finite() || !(0.0..=1.0).contains(&depth))) {return Err("invalid depth stencil clear".into())}
            let r=rs.get_mut(&target).ok_or("unknown depth target")?;
            let (w,h)=depth_image(r)?;if !matches!(r.kind,ResourceKind::Texture {format:20,..}) {return Err("expected depth stencil target".into())}
            let size=w as usize*h as usize*4;
            if flags & 1 != 0 {for bytes in r.bytes[..size].chunks_exact_mut(4) {bytes.copy_from_slice(&depth.to_bits().to_le_bytes());}}
            if flags & 2 != 0 {r.bytes[size..].fill(stencil);}
        },
        Command::CopyDepthStencil {src,dst,offset,pitch,width,height,plane,upload}=>{
            access(&mut rs,&mut touched,src,0x800)?;access(&mut rs,&mut touched,dst,0x400)?;
            let (tid,bid)=if upload {(dst,src)}else{(src,dst)};
            if tid==bid || plane>1 {return Err("invalid depth stencil copy".into())}
            let tex=rs.get(&tid).ok_or("unknown depth texture")?;
            if !matches!(tex.kind,ResourceKind::Texture {format:20,mips:1,..}) || depth_image(tex)?!=(width,height) {return Err("invalid depth stencil copy texture".into())}
            let row=(width as usize).checked_mul(if plane==0 {4}else{1}).ok_or("copy row overflow")?;
            let plane_offset=if plane==0 {0}else{(width as usize)*(height as usize)*4};
            if offset%512!=0 || pitch%256!=0 || (pitch as usize)<row {return Err("invalid depth stencil copy pitch".into())}
            let span=(height as usize-1).checked_mul(pitch as usize).and_then(|n|n.checked_add(row)).ok_or("copy overflow")?;
            let buf=rs.get(&bid).ok_or("unknown copy buffer")?;buffer(buf)?;let start=range(offset,span as u64,buf.bytes.len())?.start;
            let mut dest=rs.remove(&dst).ok_or("unknown destination")?;let source=rs.get(&src).ok_or("unknown source")?;
            for y in 0..height as usize {let a=plane_offset+y*row;let b=start+y*pitch as usize;if upload {dest.bytes[a..a+row].copy_from_slice(&source.bytes[b..b+row]);}else{dest.bytes[b..b+row].copy_from_slice(&source.bytes[a..a+row]);}}
            rs.insert(dst,dest);
        },
        Command::CopyBuffer {src,dst,src_offset,dst_offset,size}=>{access(&mut rs,&mut touched,src,0x800)?;access(&mut rs,&mut touched,dst,0x400)?;let s=rs.get(&src).ok_or("unknown source")?;buffer(s)?;let a=range(src_offset,size,s.bytes.len())?;let d=rs.get(&dst).ok_or("unknown destination")?;buffer(d)?;let b=range(dst_offset,size,d.bytes.len())?;if src==dst {rs.get_mut(&dst).unwrap().bytes.copy_within(a,b.start);} else {let mut d=rs.remove(&dst).unwrap();d.bytes[b].copy_from_slice(&rs.get(&src).unwrap().bytes[a]);rs.insert(dst,d);}},
    }}
    for (id,decay) in touched {if decay {rs.get_mut(&id).unwrap().state=0;}}
    let mut out:Vec<_>=rs.into_values().collect();out.sort_unstable_by_key(|r|r.id);Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_generations_follow_command_order_and_resource_identity() {
        let mut generations = HashMap::new();
        let draw = Draw {
            target: 1, pipeline: 1, vertex_buffer: 2, vertex_offset: 0, vertex_size: 0,
            vertex_stride: 16, vertex_count: 3, instance_count: 1, first_vertex: 0, first_instance: 0,
            viewport: [0.0; 6], scissor: [0; 4], topology: 4, constants: vec![],
            texture: Some(3), first_mip: 0, mip_count: 1, index: None, depth: Some(4),
            linear_filter: false, constant_slot: None, texture_slot: None, sampler_slot: None,
            vertices: vec![], textures: vec![], samplers: vec![],
            extra_targets: vec![],
        };
        mark_writes(&mut generations, &Command::Draw(draw), 1);
        assert_eq!(generations.get(&1), Some(&1));
        assert_eq!(generations.get(&4), Some(&1));
        assert!(!generations.contains_key(&2));
        assert!(!generations.contains_key(&3));

        let commands = [
            Command::Clear { target: 3, color: [0.0; 4] },
            Command::CopyTexture(CopyTexture { src: 1, dst: 3, offset: 0, pitch: 256, width: 1, height: 1, format: 28, mip: 0, buffer_to_texture: true }),
            Command::ClearDepth { target: 3, depth: 1.0 },
            Command::ClearStencil { target: 3, flags: 2, depth: 1.0, stencil: 0 },
            Command::CopyDepthStencil { src: 1, dst: 3, offset: 0, pitch: 256, width: 1, height: 1, plane: 0, upload: true },
            Command::CopyBuffer { src: 1, dst: 3, src_offset: 0, dst_offset: 0, size: 1 },
        ];
        for (index, command) in commands.iter().enumerate() {
            mark_writes(&mut generations, command, index + 2);
            assert_eq!(generations.get(&3), Some(&(index + 2)));
            assert_eq!(generations.get(&1), Some(&1));
        }
        mark_writes(&mut generations, &Command::Barrier { id: 3, before: 0, after: 0 }, 8);
        mark_writes(&mut generations, &Command::SetStencilRef(2), 9);
        assert_eq!(generations.get(&3), Some(&7));
    }
}
