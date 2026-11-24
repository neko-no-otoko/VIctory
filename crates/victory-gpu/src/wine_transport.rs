//! Copied, bounded, little-endian Wine graphics batch protocol (version 5).
use std::collections::HashSet;

pub const MAX_PAYLOAD: usize = 64 * 1024 * 1024;
const MAX_ITEMS: usize = 4096;
const MAX_RESOURCE: usize = 16 * 1024 * 1024;
const MAX_SHADER: usize = 4 * 1024 * 1024;

/// Returns the packed row byte length and row count for a supported texture format.
pub fn block_geometry(format:u32,width:u32,height:u32)->Option<(usize,usize)> {
    if width==0 || height==0 {return None}
    let (block_width,block_height,bytes)=match format {
        28|29|40|87|91=>(1usize,1usize,4usize),
        71|72=>(4,4,8),
        77|78=>(4,4,16),
        _=>return None,
    };
    let columns=(width as usize-1)/block_width+1;
    let rows=(height as usize-1)/block_height+1;
    Some((columns.checked_mul(bytes)?,rows))
}

/// Returns tightly packed mip dimensions, offset and length for a supported format.
pub fn mip_layout(format:u32,width:u32,height:u32,mips:u32,mip:u32)->Option<(u32,u32,usize,usize)> {
    if width==0 || height==0 || mips==0 || mips>u32::BITS-width.max(height).leading_zeros() || mip>=mips {return None}
    let mut offset=0usize;
    let mut result=None;
    for level in 0..mips {
        let w=(width>>level).max(1);
        let h=(height>>level).max(1);
        let (row_bytes,rows)=block_geometry(format,w,h)?;
        let len=row_bytes.checked_mul(rows)?;
        if level==mip {result=Some((w,h,offset,len));}
        offset=offset.checked_add(len)?;
    }
    result
}

/// Returns the byte length of all tightly packed mip levels.
pub fn texture_size(format:u32,width:u32,height:u32,mips:u32)->Option<usize> {
    if format == 20 {
        if width == 0 || height == 0 || mips != 1 { return None; }
        return (width as usize).checked_mul(height as usize)?.checked_mul(5);
    }
    let (_,_,offset,len)=mip_layout(format,width,height,mips,mips.checked_sub(1)?)?;
    offset.checked_add(len)
}

#[derive(Clone, Debug)]
pub struct Batch { pub resources: Vec<Resource>, pub pipelines: Vec<Pipeline>, pub commands: Vec<Command> }
#[derive(Clone, Debug)]
pub struct Resource { pub id: u64, pub state: u32, pub kind: ResourceKind, pub bytes: Vec<u8> }
#[derive(Clone, Copy, Debug)]
pub enum ResourceKind { Buffer, Texture { width: u32, height: u32, format: u32, mips: u32 } }
#[derive(Clone, Debug)]
pub enum Command {
    Clear { target: u64, color: [f32; 4] }, Draw(Draw),
    Barrier { id: u64, before: u32, after: u32 }, CopyTexture(CopyTexture),
    CopyBuffer { src: u64, dst: u64, src_offset: u64, dst_offset: u64, size: u64 },
    ClearDepth { target:u64, depth:f32 },
    SetStencilRef(u32),
    ClearStencil { target:u64, flags:u32, depth:f32, stencil:u8 },
    CopyDepthStencil { src:u64, dst:u64, offset:u64, pitch:u32, width:u32, height:u32, plane:u32, upload:bool },
}
#[derive(Clone, Debug)]
pub struct Draw {
    pub target: u64, pub pipeline:u64,
    pub vertex_buffer: u64, pub vertex_offset: u64, pub vertex_size: u32,
    pub vertex_stride:u32,
    pub vertex_count: u32, pub instance_count: u32, pub first_vertex: u32, pub first_instance: u32,
    pub viewport: [f32; 6], pub scissor: [i32; 4], pub topology: u32,
    pub constants: Vec<u32>, pub texture: Option<u64>, pub first_mip:u32, pub mip_count:u32,
    pub index: Option<Index>, pub depth: Option<u64>, pub linear_filter: bool,
    pub constant_slot:Option<u32>, pub texture_slot:Option<u32>, pub sampler_slot:Option<u32>,
    /// Explicit vertex views for the extended input-layout protocol; empty for legacy draws.
    pub vertices: Vec<VertexView>,
}
#[derive(Clone, Debug)]
pub struct VertexView { pub slot:u32, pub buffer:u64, pub offset:u64, pub size:u32, pub stride:u32 }
#[derive(Clone, Debug)]
pub struct VertexInput {
    pub semantic:String, pub semantic_index:u32, pub format:u32, pub slot:u32,
    pub offset:u32, pub per_instance:bool, pub step_rate:u32,
}

pub fn vertex_format_size(format:u32) -> Option<u32> {
    match format { 2=>Some(16), 16=>Some(8), 34|35=>Some(4), _=>None }
}

pub fn validate_vertex_inputs(inputs:&[VertexInput]) -> Result<(),String> {
    if inputs.is_empty() || inputs.len()>16 {return Err("invalid vertex input count".into())}
    for (i,a) in inputs.iter().enumerate() {
        let size=vertex_format_size(a.format).ok_or("unsupported vertex format")?;
        let align=if matches!(a.format,34|35) {2} else {4};
        if a.slot>=16 || a.semantic.is_empty() || a.semantic.len()>64
            || !a.semantic.bytes().all(|c| c.is_ascii_alphanumeric() || c==b'_')
            || a.offset%align!=0 || a.offset.checked_add(size).is_none_or(|v|v>2048)
            || (!a.per_instance && a.step_rate!=0) || (a.per_instance && a.step_rate==0)
        {return Err("invalid vertex input".into())}
        for b in &inputs[..i] {
            if (a.semantic_index==b.semantic_index && a.semantic.eq_ignore_ascii_case(&b.semantic))
                || (a.slot==b.slot && (a.per_instance!=b.per_instance || a.step_rate!=b.step_rate))
            {return Err("duplicate semantic or inconsistent vertex slot".into())}
        }
    }
    Ok(())
}
#[derive(Clone, Debug)]
pub struct Index { pub buffer:u64, pub offset:u64, pub size:u32, pub format:u32, pub first:u32, pub base_vertex:i32 }
#[derive(Clone, Debug)]
pub struct PipelineDepth { pub write:bool, pub compare:u32, pub stencil:Option<Stencil> }
#[derive(Clone, Debug)]
pub struct Stencil { pub enabled:bool, pub read_mask:u8, pub write_mask:u8, pub front:[u32;4], pub back:[u32;4] }
impl Stencil {
    pub fn valid(&self) -> bool {
        [&self.front,&self.back].iter().all(|face|
            face[..3].iter().all(|op| (1..=8).contains(op)) && (1..=8).contains(&face[3]))
    }
}
#[derive(Clone, Debug)]
pub struct Pipeline { pub id:u64, pub vertex_shader:Vec<u8>, pub pixel_shader:Vec<u8>, pub vertex_stride:u32, pub format:u32, pub depth:Option<PipelineDepth>, pub blend:Blend, pub inputs:Vec<VertexInput> }
#[derive(Clone, Debug)]
pub struct Blend { pub enabled:bool, pub write_mask:u8, pub factors:[u32;6] }
impl Blend {
    pub fn new(enabled:bool, write_mask:u8, factors:[u32;6]) -> Option<Self> {
        if write_mask > 15 { return None; }
        if !enabled {
            return Some(Self { enabled, write_mask, factors:[2,1,1,2,1,1] });
        }
        if !(1..=11).contains(&factors[0]) || !(1..=10).contains(&factors[1])
            || !(1..=5).contains(&factors[2]) || !matches!(factors[3],1|2|5|6|7|8)
            || !matches!(factors[4],1|2|5|6|7|8) || !(1..=5).contains(&factors[5])
        { return None; }
        Some(Self { enabled, write_mask, factors })
    }
    fn encode(&self) -> u32 {
        if !self.enabled { return 0; }
        match self.factors {
            [5,6,1,2,6,1] => 0,
            [5,6,1,5,6,1] => 1,
            [2,2,1,2,2,1] => 2,
            [a,b,c,d,e,f] => 0x8000_0000 | a | b<<4 | c<<8 | d<<11 | e<<15 | f<<19,
        }
    }
    fn decode(enabled:bool, mask:u8, bits:u32) -> Option<Self> {
        let factors = match bits {
            0 => [5,6,1,2,6,1],
            1 => [5,6,1,5,6,1],
            2 => [2,2,1,2,2,1],
            n if n & 0xffc0_0000 == 0x8000_0000 =>
                [n&15,(n>>4)&15,(n>>8)&7,(n>>11)&15,(n>>15)&15,(n>>19)&7],
            _ => return None,
        };
        Self::new(enabled, mask, factors)
    }
}
#[derive(Clone, Debug)]
pub struct CopyTexture {
    pub src: u64, pub dst: u64, pub offset: u64, pub pitch: u32,
    pub width: u32, pub height: u32, pub format: u32, pub mip:u32, pub buffer_to_texture: bool,
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
        validate_resource(r)?;
        self.u64(r.id); self.u32(r.state);
        match r.kind { ResourceKind::Buffer => self.u32(0), ResourceKind::Texture {width,height,format,mips} => { self.u32(1); self.u32(width); self.u32(height); self.u32(format); self.u32(mips); } }
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
    fn resource(&mut self)->Result<Resource,String> { let id=self.u64()?; let state=self.u32()?; let kind=match self.u32()? { 0=>ResourceKind::Buffer, 1=>ResourceKind::Texture {width:self.u32()?,height:self.u32()?,format:self.u32()?,mips:self.u32()?}, _=>return Err("unknown resource kind".into()) }; let resource=Resource {id,state,kind,bytes:self.bytes(MAX_RESOURCE)?}; validate_resource(&resource)?; Ok(resource) }
    fn done(&self)->Result<(),String> { if self.pos!=self.data.len() { Err("trailing wire data".into()) } else { Ok(()) } }
}
fn validate_resource(r:&Resource)->Result<(),String> {
    if let ResourceKind::Texture {width,height,format,mips}=r.kind {
        if texture_size(format,width,height,mips)!=Some(r.bytes.len()) {return Err("invalid texture format, dimensions, mip count or byte length".into())}
    }
    Ok(())
}
fn validate_commands(resources:&[Resource],commands:&[Command])->Result<(),String> {
    for command in commands {
        match command {
            Command::Draw(d)=>match d.texture {
                None if d.first_mip!=0 || d.mip_count!=0 => return Err("texture view without texture".into()),
                Some(id)=>{
                    let mips=match resources.iter().find(|r|r.id==id).map(|r|r.kind) {
                        Some(ResourceKind::Texture {mips,..})=>mips,
                        _=>return Err("unknown sampled texture".into()),
                    };
                    if d.mip_count==0 || d.first_mip.checked_add(d.mip_count).is_none_or(|end|end>mips) {return Err("invalid texture view range".into())}
                },
                None=>{},
            },
            Command::CopyTexture(c)=>{
                let id=if c.buffer_to_texture {c.dst} else {c.src};
                let (width,height,format,mips)=match resources.iter().find(|r|r.id==id).map(|r|r.kind) {
                    Some(ResourceKind::Texture {width,height,format,mips})=>(width,height,format,mips),
                    _=>return Err("unknown copy texture".into()),
                };
                if mip_layout(format,width,height,mips,c.mip).is_none() {return Err("invalid copy mip".into())}
            },
            _=>{},
        }
    }
    Ok(())
}
fn count(w:&mut Writer,n:usize)->Result<(),String> { if n>MAX_ITEMS { return Err("too many wire items".into()); } w.u32(n as u32); Ok(()) }
fn resources(w:&mut Writer,rs:&[Resource])->Result<(),String> { count(w,rs.len())?; let mut ids=HashSet::new(); for r in rs { if r.id==0 || !ids.insert(r.id) { return Err("duplicate or zero resource id".into()); } w.resource(r)?; } Ok(()) }
fn read_resources(r:&mut Reader<'_>)->Result<Vec<Resource>,String> { let n=r.count()?; let mut out=Vec::with_capacity(n); let mut ids=HashSet::new(); for _ in 0..n { let item=r.resource()?; if item.id==0 || !ids.insert(item.id) { return Err("duplicate or zero resource id".into()); } out.push(item); } Ok(out) }
fn finish(w:Writer)->Result<Vec<u8>,String> { if w.0.len()>MAX_PAYLOAD { Err("wire payload exceeds limit".into()) } else { Ok(w.0) } }
pub fn encode_results(rs:&[Resource])->Result<Vec<u8>,String> { let mut w=Writer(Vec::new()); resources(&mut w,rs)?; finish(w) }
pub fn decode_results(data:&[u8])->Result<Vec<Resource>,String> { if data.len()>MAX_PAYLOAD { return Err("wire payload exceeds limit".into()); } let mut r=Reader {data,pos:0}; let out=read_resources(&mut r)?; r.done()?; Ok(out) }
pub fn encode_batch(batch:&Batch)->Result<Vec<u8>,String> {
    validate_commands(&batch.resources,&batch.commands)?;
    let mut w=Writer(Vec::new()); resources(&mut w,&batch.resources)?; count(&mut w,batch.pipelines.len())?;
    let mut ids=HashSet::new();
    for p in &batch.pipelines {
        if p.id==0 || !ids.insert(p.id) {return Err("duplicate pipeline id".into())}
        w.u64(p.id);w.bytes(&p.vertex_shader,MAX_SHADER)?;w.bytes(&p.pixel_shader,MAX_SHADER)?;
        if p.vertex_stride & 0x8000_0000 != 0 {return Err("invalid vertex stride".into())}
        w.u32(p.vertex_stride | if p.inputs.is_empty(){0}else{0x8000_0000});w.u32(p.format);
        if let Some(z)=&p.depth {
            w.u32(if z.stencil.is_some(){2}else{1});w.u32(z.write as u32);w.u32(z.compare);
            if let Some(s)=&z.stencil {
                if s.enabled && !s.valid() {return Err("invalid stencil state".into())}
                w.u32(s.enabled as u32);w.u32(s.read_mask as u32);w.u32(s.write_mask as u32);
                for v in s.front.into_iter().chain(s.back) {w.u32(v);}
            }
        } else {w.u32(0);}
        if Blend::new(p.blend.enabled,p.blend.write_mask,p.blend.factors).is_none() {return Err("invalid blend state".into())}
        w.u32(p.blend.enabled as u32);w.u32(p.blend.write_mask as u32);w.u32(p.blend.encode());
        if !p.inputs.is_empty() {
            validate_vertex_inputs(&p.inputs)?;
            w.u32(p.inputs.len() as u32);
            for a in &p.inputs {
                w.bytes(a.semantic.as_bytes(),64)?;
                for v in [a.semantic_index,a.format,a.slot,a.offset,a.per_instance as u32,a.step_rate] {w.u32(v);}
            }
        }
    }
    count(&mut w,batch.commands.len())?;
    for c in &batch.commands { match c {
        Command::Clear {target,color} => {w.u32(0);w.u64(*target);for v in color {w.u32(v.to_bits());}},
        Command::Draw(d) => {w.u32(if d.vertices.is_empty(){1}else{9});w.u64(d.target);w.u64(d.pipeline);w.u64(d.vertex_buffer);w.u64(d.vertex_offset);for v in [d.vertex_size,d.vertex_count,d.instance_count,d.first_vertex,d.first_instance] {w.u32(v);} for v in d.viewport {w.u32(v.to_bits());} for v in d.scissor {w.u32(v as u32);} w.u32(d.topology); if d.constants.len()>64 {return Err("too many constants".into());} w.u32(d.constants.len() as u32);for v in &d.constants {w.u32(*v);} w.u64(d.texture.unwrap_or(0));w.u32(d.first_mip);w.u32(d.mip_count);
            if d.texture==Some(0) || d.depth==Some(0) || [d.constant_slot,d.texture_slot,d.sampler_slot].contains(&Some(u32::MAX)) {return Err("invalid optional identifier or slot".into())}
            if let Some(i)=&d.index {w.u32(1);w.u64(i.buffer);w.u64(i.offset);for v in [i.size,i.format,i.first,i.base_vertex as u32] {w.u32(v);}} else {w.u32(0);}
            w.u64(d.depth.unwrap_or(0));w.u32(d.linear_filter as u32);
            w.u32(d.vertex_stride);for slot in [d.constant_slot,d.texture_slot,d.sampler_slot] {w.u32(slot.unwrap_or(u32::MAX));}
            if !d.vertices.is_empty() {
                if d.vertices.len()>16 {return Err("too many vertex views".into())}
                w.u32(d.vertices.len() as u32);
                let mut slots=0u32;
                for v in &d.vertices {
                    if v.slot>=16 || v.buffer==0 || v.size==0 || v.stride==0 || slots & (1<<v.slot)!=0 {return Err("invalid vertex view".into())}
                    slots|=1<<v.slot;w.u32(v.slot);w.u64(v.buffer);w.u64(v.offset);w.u32(v.size);w.u32(v.stride);
                }
            }
        },
        Command::Barrier {id,before,after} => {w.u32(2);w.u64(*id);w.u32(*before);w.u32(*after);},
        Command::CopyTexture(c) => {w.u32(3);w.u64(c.src);w.u64(c.dst);w.u64(c.offset);for v in [c.pitch,c.width,c.height,c.format,c.mip,c.buffer_to_texture as u32] {w.u32(v);}},
        Command::CopyBuffer {src,dst,src_offset,dst_offset,size} => {w.u32(4);for v in [src,dst,src_offset,dst_offset,size] {w.u64(*v);}},
        Command::ClearDepth {target,depth}=>{w.u32(5);w.u64(*target);w.u32(depth.to_bits());},
        Command::SetStencilRef(value)=>{w.u32(6);w.u32(*value);},
        Command::ClearStencil {target,flags,depth,stencil}=>{w.u32(7);w.u64(*target);w.u32(*flags);w.u32(depth.to_bits());w.u32(u32::from(*stencil));},
        Command::CopyDepthStencil {src,dst,offset,pitch,width,height,plane,upload}=>{w.u32(8);w.u64(*src);w.u64(*dst);w.u64(*offset);for v in [*pitch,*width,*height,*plane,*upload as u32] {w.u32(v);}},
    }} finish(w)
}
pub fn decode_batch(data:&[u8])->Result<Batch,String> {
    if data.len()>MAX_PAYLOAD {return Err("wire payload exceeds limit".into());} let mut r=Reader {data,pos:0}; let resources=read_resources(&mut r)?;
    let n=r.count()?;let mut pipelines=Vec::with_capacity(n);let mut ids=HashSet::new();
    for _ in 0..n {
        let id=r.u64()?;if id==0 || !ids.insert(id) {return Err("duplicate pipeline id".into())}
        let vertex_shader=r.bytes(MAX_SHADER)?;let pixel_shader=r.bytes(MAX_SHADER)?;
        let raw_stride=r.u32()?;let extended=raw_stride & 0x8000_0000!=0;let vertex_stride=raw_stride & 0x7fff_ffff;
        let format=r.u32()?;
        let depth=match r.u32()? {
            0=>None,k @ (1|2)=>{
                let write=r.boolean()?;let compare=r.u32()?;
                let stencil=if k==2 {
                    let enabled=r.boolean()?;let read_mask=u8::try_from(r.u32()?).map_err(|_|"invalid stencil mask")?;
                    let write_mask=u8::try_from(r.u32()?).map_err(|_|"invalid stencil mask")?;
                    let mut front=[0;4];let mut back=[0;4];for v in front.iter_mut().chain(back.iter_mut()) {*v=r.u32()?;}
                    let s=Stencil {enabled,read_mask,write_mask,front,back};
                    if enabled && !s.valid() {return Err("invalid stencil state".into())} Some(s)
                } else {None};
                Some(PipelineDepth {write,compare,stencil})
            },_=>return Err("invalid depth state".into())
        };
        let enabled=r.boolean()?;let mask=r.u32()?;if mask>15 {return Err("invalid blend mask".into())}
        let blend=Blend::decode(enabled,mask as u8,r.u32()?).ok_or("invalid blend flags")?;
        let mut inputs=Vec::new();
        if extended {
            let n=r.count()?;if n==0 || n>16 {return Err("invalid vertex input count".into())}
            for _ in 0..n {
                let semantic=String::from_utf8(r.bytes(64)?).map_err(|_|"invalid vertex semantic")?;
                inputs.push(VertexInput {semantic,semantic_index:r.u32()?,format:r.u32()?,slot:r.u32()?,offset:r.u32()?,per_instance:r.boolean()?,step_rate:r.u32()?});
            }
            validate_vertex_inputs(&inputs)?;
        }
        pipelines.push(Pipeline {id,vertex_shader,pixel_shader,vertex_stride,format,depth,blend,inputs});
    }
    let n=r.count()?; let mut commands=Vec::with_capacity(n);
    for _ in 0..n { commands.push(match r.u32()? {
        0=> {let target=r.u64()?;let mut color=[0.;4];for v in &mut color {*v=f32::from_bits(r.u32()?);} Command::Clear {target,color}},
        kind @ (1|9)=> {let target=r.u64()?;let pipeline=r.u64()?;let vertex_buffer=r.u64()?;let vertex_offset=r.u64()?;let vertex_size=r.u32()?;let vertex_count=r.u32()?;let instance_count=r.u32()?;let first_vertex=r.u32()?;let first_instance=r.u32()?;let mut viewport=[0.;6];for v in &mut viewport {*v=f32::from_bits(r.u32()?);} let mut scissor=[0;4];for v in &mut scissor {*v=r.u32()? as i32;} let topology=r.u32()?;let count=r.u32()? as usize;if count>64 {return Err("too many constants".into());} let mut constants=Vec::with_capacity(count);for _ in 0..count {constants.push(r.u32()?);} let texture=match r.u64()? {0=>None,id=>Some(id)};let first_mip=r.u32()?;let mip_count=r.u32()?;
            let index=if r.boolean()? {Some(Index {buffer:r.u64()?,offset:r.u64()?,size:r.u32()?,format:r.u32()?,first:r.u32()?,base_vertex:r.u32()? as i32})} else {None};
            let depth=match r.u64()? {0=>None,id=>Some(id)};let linear_filter=r.boolean()?;let vertex_stride=r.u32()?;let mut slot=|| -> Result<Option<u32>,String> {Ok(match r.u32()? {u32::MAX=>None,v=>Some(v)})};let constant_slot=slot()?;let texture_slot=slot()?;let sampler_slot=slot()?;
            let mut vertices=Vec::new();
            if kind==9 {
                let n=r.count()?;if n==0 || n>16 {return Err("invalid vertex view count".into())}
                let mut slots=0u32;
                for _ in 0..n {
                    let v=VertexView {slot:r.u32()?,buffer:r.u64()?,offset:r.u64()?,size:r.u32()?,stride:r.u32()?};
                    if v.slot>=16 || v.buffer==0 || v.size==0 || v.stride==0 || slots & (1<<v.slot)!=0 {return Err("invalid vertex view".into())}
                    slots|=1<<v.slot;vertices.push(v);
                }
            }
            Command::Draw(Draw {target,pipeline,vertex_buffer,vertex_offset,vertex_size,vertex_stride,vertex_count,instance_count,first_vertex,first_instance,viewport,scissor,topology,constants,texture,first_mip,mip_count,index,depth,linear_filter,constant_slot,texture_slot,sampler_slot,vertices})},
        2=>Command::Barrier {id:r.u64()?,before:r.u32()?,after:r.u32()?},
        3=> {let src=r.u64()?;let dst=r.u64()?;let offset=r.u64()?;let pitch=r.u32()?;let width=r.u32()?;let height=r.u32()?;let format=r.u32()?;let mip=r.u32()?;let buffer_to_texture=match r.u32()? {0=>false,1=>true,_=>return Err("invalid copy direction".into())};Command::CopyTexture(CopyTexture {src,dst,offset,pitch,width,height,format,mip,buffer_to_texture})},
        4=>Command::CopyBuffer {src:r.u64()?,dst:r.u64()?,src_offset:r.u64()?,dst_offset:r.u64()?,size:r.u64()?},
        5=>Command::ClearDepth {target:r.u64()?,depth:f32::from_bits(r.u32()?)},
        6=>Command::SetStencilRef(r.u32()?),
        7=>Command::ClearStencil {target:r.u64()?,flags:r.u32()?,depth:f32::from_bits(r.u32()?),stencil:u8::try_from(r.u32()?).map_err(|_|"invalid stencil clear")?},
        8=>Command::CopyDepthStencil {src:r.u64()?,dst:r.u64()?,offset:r.u64()?,pitch:r.u32()?,width:r.u32()?,height:r.u32()?,plane:r.u32()?,upload:r.boolean()?},
        _=>return Err("unknown wire command".into()),
    }); } r.done()?;validate_commands(&resources,&commands)?;Ok(Batch {resources,pipelines,commands})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_wire_legacy_and_extended() {
        for (wire, factors) in [
            (0,[5,6,1,2,6,1]), (1,[5,6,1,5,6,1]), (2,[2,2,1,2,2,1]),
        ] {
            let state=Blend::decode(true,15,wire).unwrap();
            assert_eq!(state.factors,factors);
            assert_eq!(state.encode(),wire);
        }
        let state=Blend::new(true,5,[3,10,2,5,8,3]).unwrap();
        assert_eq!(Blend::decode(true,5,state.encode()).unwrap().factors,state.factors);
        assert!(Blend::decode(true,15,3).is_none());
        assert!(Blend::decode(true,15,0x8040_0000).is_none());
        assert!(Blend::new(true,15,[16,2,1,2,2,1]).is_none());
        assert!(Blend::new(true,15,[2,2,1,3,2,1]).is_none());
        assert!(Blend::new(false,5,[u32::MAX;6]).is_some());
    }

    fn batch() -> Batch {
        Batch {
            resources: vec![
                Resource { id:1, state:4, kind:ResourceKind::Buffer, bytes:vec![1,2,3,4] },
                Resource { id:2, state:8, kind:ResourceKind::Texture {width:1,height:1,format:28,mips:1}, bytes:vec![4,3,2,1] },
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
            depth:Some(PipelineDepth {write:true,compare:4,stencil:None}),blend:Blend::new(true,15,[3,7,2,5,8,3]).unwrap(),inputs:vec![]});
        b.commands.push(Command::Draw(Draw {target:2,pipeline:7,vertex_buffer:1,vertex_offset:0,vertex_size:4,vertex_stride:16,
            vertex_count:3,instance_count:1,first_vertex:0,first_instance:0,viewport:[0.,0.,1.,1.,0.,1.],scissor:[0,0,1,1],
            topology:4,constants:vec![10,20],texture:Some(2),first_mip:0,mip_count:1,index:Some(Index {buffer:1,offset:0,size:4,format:57,first:0,base_vertex:-1}),
            depth:Some(3),linear_filter:true,constant_slot:Some(3),texture_slot:Some(2),sampler_slot:Some(1),vertices:vec![]}));
        let encoded=encode_batch(&b).unwrap();
        assert_eq!(encode_batch(&decode_batch(&encoded).unwrap()).unwrap(),encoded);
        b.pipelines.push(b.pipelines[0].clone());
        assert!(encode_batch(&b).is_err());
    }

    #[test]
    fn vertex_input_wire_round_trip() {
        let mut b=batch();
        b.pipelines.push(Pipeline {id:7,vertex_shader:vec![1],pixel_shader:vec![2],vertex_stride:24,format:28,
            depth:None,blend:Blend::new(false,15,[0;6]).unwrap(),inputs:vec![
                VertexInput {semantic:"TEXCOORD".into(),semantic_index:0,format:34,slot:1,offset:0,per_instance:true,step_rate:2},
                VertexInput {semantic:"POSITION".into(),semantic_index:0,format:2,slot:0,offset:4,per_instance:false,step_rate:0},
            ]});
        b.commands.push(Command::Draw(Draw {target:2,pipeline:7,vertex_buffer:1,vertex_offset:0,vertex_size:4,vertex_stride:16,
            vertex_count:3,instance_count:3,first_vertex:0,first_instance:2,viewport:[0.,0.,1.,1.,0.,1.],scissor:[0,0,1,1],
            topology:4,constants:vec![],texture:None,first_mip:0,mip_count:0,index:None,depth:None,linear_filter:false,
            constant_slot:None,texture_slot:None,sampler_slot:None,
            vertices:vec![VertexView {slot:0,buffer:1,offset:0,size:4,stride:16},VertexView {slot:1,buffer:1,offset:0,size:4,stride:4}]}));
        let bytes=encode_batch(&b).unwrap();
        assert_eq!(encode_batch(&decode_batch(&bytes).unwrap()).unwrap(),bytes);
        b.pipelines[0].inputs[0].step_rate=0;
        assert!(encode_batch(&b).is_err());
    }

    #[test]
    fn stencil_wire_round_trip() {
        let mut b=batch();
        b.resources.push(Resource {id:3,state:0x10,kind:ResourceKind::Texture {width:1,height:1,format:20,mips:1},bytes:vec![0;5]});
        b.pipelines.push(Pipeline {id:7,vertex_shader:vec![1],pixel_shader:vec![2],vertex_stride:16,format:28,
            depth:Some(PipelineDepth {write:true,compare:2,stencil:Some(Stencil {enabled:true,read_mask:0xf0,write_mask:0x0f,front:[1,2,3,4],back:[5,6,7,8]})}),
            blend:Blend::new(false,15,[0;6]).unwrap(),inputs:vec![]});
        b.commands.extend([Command::SetStencilRef(0xa5),Command::ClearStencil {target:3,flags:3,depth:0.5,stencil:0xa0},
            Command::CopyDepthStencil {src:3,dst:1,offset:0,pitch:256,width:1,height:1,plane:1,upload:false}]);
        let encoded=encode_batch(&b).unwrap();
        assert_eq!(encode_batch(&decode_batch(&encoded).unwrap()).unwrap(),encoded);
        b.pipelines[0].depth.as_mut().unwrap().stencil.as_mut().unwrap().front[0]=9;
        assert!(encode_batch(&b).is_err());
    }

    #[test]
    fn mip_layout_odd_dimensions_and_invalid_levels() {
        assert_eq!(mip_layout(28,7,3,3,0),Some((7,3,0,84)));
        assert_eq!(mip_layout(28,7,3,3,1),Some((3,1,84,12)));
        assert_eq!(mip_layout(28,7,3,3,2),Some((1,1,96,4)));
        assert_eq!(texture_size(28,7,3,3),Some(100));
        assert_eq!(mip_layout(28,1,5,3,1),Some((1,2,20,8)));
        for (w,h,n,level) in [(0,1,1,0),(1,0,1,0),(1,1,0,0),(7,3,4,0),(7,3,3,3)] {
            assert_eq!(mip_layout(28,w,h,n,level),None);
        }
        assert_eq!(texture_size(28,u32::MAX,u32::MAX,1),None);
    }

    #[test]
    fn format_block_geometry_and_mip_offsets() {
        for format in [28,29,40,87,91] {
            assert_eq!(block_geometry(format,7,3),Some((28,3)));
            assert_eq!(texture_size(format,7,3,3),Some(100));
        }
        for (format,block) in [(71,8),(72,8),(77,16),(78,16)] {
            assert_eq!(block_geometry(format,9,5),Some((3*block,2)));
            assert_eq!(mip_layout(format,8,8,4,0),Some((8,8,0,4*block)));
            assert_eq!(mip_layout(format,8,8,4,1),Some((4,4,4*block,block)));
            assert_eq!(mip_layout(format,8,8,4,2),Some((2,2,5*block,block)));
            assert_eq!(mip_layout(format,8,8,4,3),Some((1,1,6*block,block)));
            assert_eq!(texture_size(format,8,8,4),Some(7*block));
            assert_eq!(block_geometry(format,0,1),None);
            assert_eq!(block_geometry(format,1,0),None);
            assert_eq!(mip_layout(format,8,8,5,0),None);
            assert_eq!(mip_layout(format,8,8,4,4),None);
            assert_eq!(texture_size(format,8,8,0),None);
        }
        for format in [0,27,70,73,76,79,u32::MAX] {
            assert_eq!(block_geometry(format,4,4),None);
            assert_eq!(mip_layout(format,4,4,1,0),None);
            assert_eq!(texture_size(format,4,4,1),None);
        }
        assert_eq!(texture_size(77,u32::MAX,u32::MAX,1),None);
    }

    #[test]
    fn compressed_resources_round_trip_and_reject_invalid_lengths() {
        for (format,block) in [(71,8),(72,8),(77,16),(78,16)] {
            let resource=Resource {id:1,state:0,kind:ResourceKind::Texture {width:8,height:8,format,mips:4},bytes:vec![0x5a;7*block]};
            let encoded=encode_results(&[resource.clone()]).unwrap();
            let decoded=decode_results(&encoded).unwrap();
            assert_eq!(encode_results(&decoded).unwrap(),encoded);
            let batch=Batch {resources:vec![resource.clone()],pipelines:vec![],commands:vec![]};
            let encoded_batch=encode_batch(&batch).unwrap();
            assert_eq!(encode_batch(&decode_batch(&encoded_batch).unwrap()).unwrap(),encoded_batch);

            for len in [7*block-1,7*block+1] {
                let mut invalid=resource.clone();
                invalid.bytes.resize(len,0);
                assert!(encode_results(&[invalid.clone()]).is_err());
                assert!(encode_batch(&Batch {resources:vec![invalid],pipelines:vec![],commands:vec![]}).is_err());
                let mut wire=encoded.clone();
                wire[36..40].copy_from_slice(&(len as u32).to_le_bytes());
                wire.resize(40+len,0);
                assert!(decode_results(&wire).is_err());
            }
            let mut wire=encoded.clone();
            wire[28..32].copy_from_slice(&70u32.to_le_bytes());
            assert!(decode_results(&wire).is_err());
            let mut invalid=resource;
            invalid.kind=ResourceKind::Texture {width:8,height:8,format:70,mips:4};
            assert!(encode_results(&[invalid]).is_err());
        }
    }

    #[test]
    fn mip_views_copies_and_invalid_wire() {
        let mut b=batch();
        b.resources[1].kind=ResourceKind::Texture {width:7,height:3,format:28,mips:3};
        b.resources[1].bytes=vec![42;100];
        b.commands=vec![Command::Draw(Draw {target:2,pipeline:7,vertex_buffer:1,vertex_offset:0,vertex_size:4,vertex_stride:16,
            vertex_count:3,instance_count:1,first_vertex:0,first_instance:0,viewport:[0.,0.,1.,1.,0.,1.],scissor:[0,0,1,1],
            topology:4,constants:vec![],texture:Some(2),first_mip:1,mip_count:2,index:None,depth:None,linear_filter:false,
            constant_slot:None,texture_slot:Some(0),sampler_slot:Some(0),vertices:vec![]})];
        let encoded=encode_batch(&b).unwrap();
        for end in 0..encoded.len() {assert!(decode_batch(&encoded[..end]).is_err());}
        let decoded=decode_batch(&encoded).unwrap();
        assert!(matches!(&decoded.commands[0],Command::Draw(Draw {texture:Some(2),first_mip:1,mip_count:2,..})));
        assert_eq!(encode_batch(&decoded).unwrap(),encoded);
        let mut invalid=encoded.clone();
        let view=invalid.len()-40;
        invalid[view..view+4].copy_from_slice(&2u32.to_le_bytes());
        assert!(decode_batch(&invalid).is_err());
        if let Command::Draw(d)=&mut b.commands[0] {d.first_mip=u32::MAX;}
        assert!(encode_batch(&b).is_err());
        if let Command::Draw(d)=&mut b.commands[0] {d.texture=None;d.first_mip=0;d.mip_count=1;}
        assert!(encode_batch(&b).is_err());
        b.commands=vec![Command::CopyTexture(CopyTexture {src:1,dst:2,offset:0,pitch:12,width:3,height:1,format:28,mip:1,buffer_to_texture:true})];
        let encoded=encode_batch(&b).unwrap();
        assert!(matches!(&decode_batch(&encoded).unwrap().commands[0],Command::CopyTexture(CopyTexture {mip:1,..})));
        let mut invalid=encoded.clone();
        let pos=invalid.len()-8;
        invalid[pos..pos+4].copy_from_slice(&3u32.to_le_bytes());
        assert!(decode_batch(&invalid).is_err());
        b.resources[1].bytes.pop();
        assert!(encode_batch(&b).is_err());
        assert!(encode_results(&b.resources).is_err());
    }
}
