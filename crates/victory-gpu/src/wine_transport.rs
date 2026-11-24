//! Copied, bounded, little-endian Wine graphics batch protocol (version 3).
use std::collections::HashSet;

pub const MAX_PAYLOAD: usize = 64 * 1024 * 1024;
const MAX_ITEMS: usize = 4096;
const MAX_RESOURCE: usize = 16 * 1024 * 1024;
const MAX_SHADER: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Batch { pub resources: Vec<Resource>, pub pipelines: Vec<Pipeline>, pub commands: Vec<Command> }
#[derive(Clone, Debug)]
pub struct Resource { pub id: u64, pub state: u32, pub kind: ResourceKind, pub bytes: Vec<u8> }
#[derive(Clone, Copy, Debug)]
pub enum ResourceKind { Buffer, Texture { width: u32, height: u32, format: u32 } }
#[derive(Clone, Debug)]
pub enum Command {
    Clear { target: u64, color: [f32; 4] }, Draw(Draw),
    Barrier { id: u64, before: u32, after: u32 }, CopyTexture(CopyTexture),
    CopyBuffer { src: u64, dst: u64, src_offset: u64, dst_offset: u64, size: u64 },
    ClearDepth { target:u64, depth:f32 },
}
#[derive(Clone, Debug)]
pub struct Draw {
    pub target: u64, pub pipeline:u64,
    pub vertex_buffer: u64, pub vertex_offset: u64, pub vertex_size: u32,
    pub vertex_stride:u32,
    pub vertex_count: u32, pub instance_count: u32, pub first_vertex: u32, pub first_instance: u32,
    pub viewport: [f32; 6], pub scissor: [i32; 4], pub topology: u32,
    pub constants: Vec<u32>, pub texture: Option<u64>,
    pub index: Option<Index>, pub depth: Option<u64>, pub linear_filter: bool,
    pub constant_slot:Option<u32>, pub texture_slot:Option<u32>, pub sampler_slot:Option<u32>,
}
#[derive(Clone, Debug)]
pub struct Index { pub buffer:u64, pub offset:u64, pub size:u32, pub format:u32, pub first:u32, pub base_vertex:i32 }
#[derive(Clone, Debug)]
pub struct PipelineDepth { pub write:bool, pub compare:u32 }
#[derive(Clone, Debug)]
pub struct Pipeline { pub id:u64, pub vertex_shader:Vec<u8>, pub pixel_shader:Vec<u8>, pub vertex_stride:u32, pub format:u32, pub depth:Option<PipelineDepth>, pub blend:Blend }
#[derive(Clone, Debug)]
pub struct Blend { pub enabled:bool, pub write_mask:u8, pub alpha_source:bool }
#[derive(Clone, Debug)]
pub struct CopyTexture {
    pub src: u64, pub dst: u64, pub offset: u64, pub pitch: u32,
    pub width: u32, pub height: u32, pub format: u32, pub buffer_to_texture: bool,
}

struct Writer(Vec<u8>);
impl Writer {
    fn u32(&mut self, n:u32) { self.0.extend_from_slice(&n.to_le_bytes()); }
    fn u64(&mut self, n:u64) { self.0.extend_from_slice(&n.to_le_bytes()); }
    fn bytes(&mut self, b:&[u8], max:usize) -> Result<(),String> {
        if b.len()>max || self.0.len().checked_add(4).and_then(|n| n.checked_add(b.len())).is_none_or(|n| n>MAX_PAYLOAD) { return Err("wire field exceeds limit".into()); }
        self.u32(b.len() as u32); self.0.extend_from_slice(b); Ok(())
    }
    fn resource(&mut self, r:&Resource) -> Result<(),String> {
        self.u64(r.id); self.u32(r.state);
        match r.kind { ResourceKind::Buffer => self.u32(0), ResourceKind::Texture {width,height,format} => { self.u32(1); self.u32(width); self.u32(height); self.u32(format); } }
        self.bytes(&r.bytes,MAX_RESOURCE)
    }
}
struct Reader<'a> { data:&'a [u8], pos:usize }
impl<'a> Reader<'a> {
    fn take(&mut self, n:usize)->Result<&'a [u8],String> { let end=self.pos.checked_add(n).ok_or("wire offset overflow")?; let b=self.data.get(self.pos..end).ok_or("truncated wire data")?; self.pos=end; Ok(b) }
    fn u32(&mut self)->Result<u32,String> { Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap())) }
    fn u64(&mut self)->Result<u64,String> { Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap())) }
    fn boolean(&mut self)->Result<bool,String> {match self.u32()? {0=>Ok(false),1=>Ok(true),_=>Err("invalid wire boolean".into())}}
    fn bytes(&mut self,max:usize)->Result<Vec<u8>,String> { let n=self.u32()? as usize; if n>max { return Err("wire field exceeds limit".into()); } Ok(self.take(n)?.to_vec()) }
    fn count(&mut self)->Result<usize,String> { let n=self.u32()? as usize; if n>MAX_ITEMS { return Err("too many wire items".into()); } Ok(n) }
    fn resource(&mut self)->Result<Resource,String> { let id=self.u64()?; let state=self.u32()?; let kind=match self.u32()? { 0=>ResourceKind::Buffer, 1=>ResourceKind::Texture {width:self.u32()?,height:self.u32()?,format:self.u32()?}, _=>return Err("unknown resource kind".into()) }; Ok(Resource {id,state,kind,bytes:self.bytes(MAX_RESOURCE)?}) }
    fn done(&self)->Result<(),String> { if self.pos!=self.data.len() { Err("trailing wire data".into()) } else { Ok(()) } }
}
fn count(w:&mut Writer,n:usize)->Result<(),String> { if n>MAX_ITEMS { return Err("too many wire items".into()); } w.u32(n as u32); Ok(()) }
fn resources(w:&mut Writer,rs:&[Resource])->Result<(),String> { count(w,rs.len())?; let mut ids=HashSet::new(); for r in rs { if r.id==0 || !ids.insert(r.id) { return Err("duplicate or zero resource id".into()); } w.resource(r)?; } Ok(()) }
fn read_resources(r:&mut Reader<'_>)->Result<Vec<Resource>,String> { let n=r.count()?; let mut out=Vec::with_capacity(n); let mut ids=HashSet::new(); for _ in 0..n { let item=r.resource()?; if item.id==0 || !ids.insert(item.id) { return Err("duplicate or zero resource id".into()); } out.push(item); } Ok(out) }
fn finish(w:Writer)->Result<Vec<u8>,String> { if w.0.len()>MAX_PAYLOAD { Err("wire payload exceeds limit".into()) } else { Ok(w.0) } }
pub fn encode_results(rs:&[Resource])->Result<Vec<u8>,String> { let mut w=Writer(Vec::new()); resources(&mut w,rs)?; finish(w) }
pub fn decode_results(data:&[u8])->Result<Vec<Resource>,String> { if data.len()>MAX_PAYLOAD { return Err("wire payload exceeds limit".into()); } let mut r=Reader {data,pos:0}; let out=read_resources(&mut r)?; r.done()?; Ok(out) }
pub fn encode_batch(batch:&Batch)->Result<Vec<u8>,String> {
    let mut w=Writer(Vec::new()); resources(&mut w,&batch.resources)?; count(&mut w,batch.pipelines.len())?;
    let mut ids=HashSet::new(); for p in &batch.pipelines {if p.id==0 || !ids.insert(p.id) {return Err("duplicate pipeline id".into())} w.u64(p.id);w.bytes(&p.vertex_shader,MAX_SHADER)?;w.bytes(&p.pixel_shader,MAX_SHADER)?;w.u32(p.vertex_stride);w.u32(p.format);if let Some(z)=&p.depth {w.u32(1);w.u32(z.write as u32);w.u32(z.compare);} else {w.u32(0);} w.u32(p.blend.enabled as u32);w.u32(p.blend.write_mask as u32);w.u32(p.blend.alpha_source as u32);}
    count(&mut w,batch.commands.len())?;
    for c in &batch.commands { match c {
        Command::Clear {target,color} => {w.u32(0);w.u64(*target);for v in color {w.u32(v.to_bits());}},
        Command::Draw(d) => {w.u32(1);w.u64(d.target);w.u64(d.pipeline);w.u64(d.vertex_buffer);w.u64(d.vertex_offset);for v in [d.vertex_size,d.vertex_count,d.instance_count,d.first_vertex,d.first_instance] {w.u32(v);} for v in d.viewport {w.u32(v.to_bits());} for v in d.scissor {w.u32(v as u32);} w.u32(d.topology); if d.constants.len()>64 {return Err("too many constants".into());} w.u32(d.constants.len() as u32);for v in &d.constants {w.u32(*v);} w.u64(d.texture.unwrap_or(0));
            if d.texture==Some(0) || d.depth==Some(0) || [d.constant_slot,d.texture_slot,d.sampler_slot].contains(&Some(u32::MAX)) {return Err("invalid optional identifier or slot".into())}
            if let Some(i)=&d.index {w.u32(1);w.u64(i.buffer);w.u64(i.offset);for v in [i.size,i.format,i.first,i.base_vertex as u32] {w.u32(v);}} else {w.u32(0);}
            w.u64(d.depth.unwrap_or(0));w.u32(d.linear_filter as u32);
            w.u32(d.vertex_stride);for slot in [d.constant_slot,d.texture_slot,d.sampler_slot] {w.u32(slot.unwrap_or(u32::MAX));}
        },
        Command::Barrier {id,before,after} => {w.u32(2);w.u64(*id);w.u32(*before);w.u32(*after);},
        Command::CopyTexture(c) => {w.u32(3);w.u64(c.src);w.u64(c.dst);w.u64(c.offset);for v in [c.pitch,c.width,c.height,c.format,c.buffer_to_texture as u32] {w.u32(v);}},
        Command::CopyBuffer {src,dst,src_offset,dst_offset,size} => {w.u32(4);for v in [src,dst,src_offset,dst_offset,size] {w.u64(*v);}},
        Command::ClearDepth {target,depth}=>{w.u32(5);w.u64(*target);w.u32(depth.to_bits());},
    }} finish(w)
}
pub fn decode_batch(data:&[u8])->Result<Batch,String> {
    if data.len()>MAX_PAYLOAD {return Err("wire payload exceeds limit".into());} let mut r=Reader {data,pos:0}; let resources=read_resources(&mut r)?;
    let n=r.count()?;let mut pipelines=Vec::with_capacity(n);let mut ids=HashSet::new();for _ in 0..n {let id=r.u64()?;if id==0 || !ids.insert(id) {return Err("duplicate pipeline id".into())} let vertex_shader=r.bytes(MAX_SHADER)?;let pixel_shader=r.bytes(MAX_SHADER)?;let vertex_stride=r.u32()?;let format=r.u32()?;let depth=if r.boolean()? {Some(PipelineDepth {write:r.boolean()?,compare:r.u32()?})} else {None};let enabled=r.boolean()?;let mask=r.u32()?;if mask>15 {return Err("invalid blend mask".into())} let blend=Blend {enabled,write_mask:mask as u8,alpha_source:r.boolean()?};pipelines.push(Pipeline {id,vertex_shader,pixel_shader,vertex_stride,format,depth,blend});}
    let n=r.count()?; let mut commands=Vec::with_capacity(n);
    for _ in 0..n { commands.push(match r.u32()? {
        0=> {let target=r.u64()?;let mut color=[0.;4];for v in &mut color {*v=f32::from_bits(r.u32()?);} Command::Clear {target,color}},
        1=> {let target=r.u64()?;let pipeline=r.u64()?;let vertex_buffer=r.u64()?;let vertex_offset=r.u64()?;let vertex_size=r.u32()?;let vertex_count=r.u32()?;let instance_count=r.u32()?;let first_vertex=r.u32()?;let first_instance=r.u32()?;let mut viewport=[0.;6];for v in &mut viewport {*v=f32::from_bits(r.u32()?);} let mut scissor=[0;4];for v in &mut scissor {*v=r.u32()? as i32;} let topology=r.u32()?;let count=r.u32()? as usize;if count>64 {return Err("too many constants".into());} let mut constants=Vec::with_capacity(count);for _ in 0..count {constants.push(r.u32()?);} let texture=match r.u64()? {0=>None,id=>Some(id)};
            let index=if r.boolean()? {Some(Index {buffer:r.u64()?,offset:r.u64()?,size:r.u32()?,format:r.u32()?,first:r.u32()?,base_vertex:r.u32()? as i32})} else {None};
            let depth=match r.u64()? {0=>None,id=>Some(id)};let linear_filter=r.boolean()?;let vertex_stride=r.u32()?;let mut slot=|| -> Result<Option<u32>,String> {Ok(match r.u32()? {u32::MAX=>None,v=>Some(v)})};let constant_slot=slot()?;let texture_slot=slot()?;let sampler_slot=slot()?;
            Command::Draw(Draw {target,pipeline,vertex_buffer,vertex_offset,vertex_size,vertex_stride,vertex_count,instance_count,first_vertex,first_instance,viewport,scissor,topology,constants,texture,index,depth,linear_filter,constant_slot,texture_slot,sampler_slot})},
        2=>Command::Barrier {id:r.u64()?,before:r.u32()?,after:r.u32()?},
        3=> {let src=r.u64()?;let dst=r.u64()?;let offset=r.u64()?;let pitch=r.u32()?;let width=r.u32()?;let height=r.u32()?;let format=r.u32()?;let buffer_to_texture=match r.u32()? {0=>false,1=>true,_=>return Err("invalid copy direction".into())};Command::CopyTexture(CopyTexture {src,dst,offset,pitch,width,height,format,buffer_to_texture})},
        4=>Command::CopyBuffer {src:r.u64()?,dst:r.u64()?,src_offset:r.u64()?,dst_offset:r.u64()?,size:r.u64()?},
        5=>Command::ClearDepth {target:r.u64()?,depth:f32::from_bits(r.u32()?)},
        _=>return Err("unknown wire command".into()),
    }); } r.done()?;Ok(Batch {resources,pipelines,commands})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch() -> Batch {
        Batch {
            resources: vec![
                Resource { id:1, state:4, kind:ResourceKind::Buffer, bytes:vec![1,2,3,4] },
                Resource { id:2, state:8, kind:ResourceKind::Texture {width:1,height:1,format:28}, bytes:vec![4,3,2,1] },
            ],
            pipelines: vec![],
            commands: vec![
                Command::Barrier {id:1,before:4,after:8},
                Command::CopyBuffer {src:1,dst:1,src_offset:0,dst_offset:2,size:2},
                Command::Clear {target:2,color:[0.0,0.5,1.0,1.0]},
            ],
        }
    }

    #[test]
    fn ordered_round_trip_and_results() {
        let bytes=encode_batch(&batch()).unwrap();
        let decoded=decode_batch(&bytes).unwrap();
        assert_eq!(encode_batch(&decoded).unwrap(),bytes);
        assert_eq!(decoded.resources[0].state,4);
        assert!(matches!(decoded.commands[0],Command::Barrier {id:1,before:4,after:8}));
        assert!(matches!(decoded.commands[1],Command::CopyBuffer {src:1,dst:1,..}));
        assert!(matches!(decoded.commands[2],Command::Clear {target:2,..}));
        let results=encode_results(&decoded.resources).unwrap();
        assert_eq!(encode_results(&decode_results(&results).unwrap()).unwrap(),results);
    }

    #[test]
    fn rejects_truncated_oversized_duplicate_and_unknown() {
        let bytes=encode_batch(&batch()).unwrap();
        for end in 0..bytes.len() {assert!(decode_batch(&bytes[..end]).is_err());}
        assert!(decode_batch(&vec![0;MAX_PAYLOAD+1]).is_err());
        let mut duplicate=batch();duplicate.resources[1].id=1;
        assert!(encode_batch(&duplicate).is_err());
        let mut duplicate_wire=bytes.clone();
        duplicate_wire[28..36].copy_from_slice(&1u64.to_le_bytes());
        assert!(decode_batch(&duplicate_wire).is_err());
        let mut unknown=Vec::new();
        for word in [0u32,0,1,99] {unknown.extend_from_slice(&word.to_le_bytes());}
        assert!(decode_batch(&unknown).is_err());
        let mut trailing=bytes;trailing.push(0);
        assert!(decode_batch(&trailing).is_err());
    }

    #[test]
    fn pipeline_and_draw_round_trip() {
        let mut b=batch();
        b.pipelines.push(Pipeline {id:7,vertex_shader:vec![1],pixel_shader:vec![2],vertex_stride:16,format:28,
            depth:Some(PipelineDepth {write:true,compare:4}),blend:Blend {enabled:true,write_mask:15,alpha_source:false}});
        b.commands.push(Command::Draw(Draw {target:2,pipeline:7,vertex_buffer:1,vertex_offset:0,vertex_size:4,vertex_stride:16,
            vertex_count:3,instance_count:1,first_vertex:0,first_instance:0,viewport:[0.,0.,1.,1.,0.,1.],scissor:[0,0,1,1],
            topology:4,constants:vec![10,20],texture:Some(2),index:Some(Index {buffer:1,offset:0,size:4,format:57,first:0,base_vertex:-1}),
            depth:Some(3),linear_filter:true,constant_slot:Some(3),texture_slot:Some(2),sampler_slot:Some(1)}));
        let encoded=encode_batch(&b).unwrap();
        assert_eq!(encode_batch(&decode_batch(&encoded).unwrap()).unwrap(),encoded);
        b.pipelines.push(b.pipelines[0].clone());
        assert!(encode_batch(&b).is_err());
    }
}
