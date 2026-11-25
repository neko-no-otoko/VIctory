//! Copied, bounded, little-endian Wine graphics batch protocol (version 5).
use std::collections::HashSet;

pub const MAX_PAYLOAD: usize = 64 * 1024 * 1024;
const MAX_ITEMS: usize = 4096;
const MAX_RESOURCE: usize = 16 * 1024 * 1024;
pub const MAX_STREAM_RESOURCE: usize = 128 * 1024 * 1024;
pub const MAX_STREAM_BATCH: usize = 256 * 1024 * 1024;
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

/// Returns the byte length of a slice-major, mip-major texture array.
pub fn array_texture_size(format:u32,width:u32,height:u32,mips:u32,layers:u32)->Option<usize> {
    if layers==0 || format==20 {return None}
    texture_size(format,width,height,mips)?.checked_mul(layers as usize)
}

/// Returns dimensions, byte offset and byte length of an array subresource.
pub fn array_mip_layout(format:u32,width:u32,height:u32,mips:u32,layers:u32,slice:u32,mip:u32)->Option<(u32,u32,usize,usize)> {
    if slice>=layers {return None}
    let total=array_texture_size(format,width,height,mips,layers)?;
    let stride=total/(layers as usize);
    let (w,h,offset,len)=mip_layout(format,width,height,mips,mip)?;
    Some((w,h,stride.checked_mul(slice as usize)?.checked_add(offset)?,len))
}

/// Returns tightly packed volume mip dimensions, offset and length.
pub fn volume_mip_layout(format:u32,width:u32,height:u32,depth:u32,mips:u32,mip:u32)->Option<(u32,u32,u32,usize,usize)> {
    if !matches!(format,28|29|87|91) || width==0 || height==0 || depth==0 || mips==0 || mips>u32::BITS-width.max(height).max(depth).leading_zeros() || mip>=mips {return None}
    let mut offset=0usize;
    let mut result=None;
    for level in 0..mips {
        let w=(width>>level).max(1);
        let h=(height>>level).max(1);
        let d=(depth>>level).max(1);
        let (row,rows)=block_geometry(format,w,h)?;
        let len=row.checked_mul(rows)?.checked_mul(d as usize)?;
        if level==mip {result=Some((w,h,d,offset,len));}
        offset=offset.checked_add(len)?;
    }
    result
}

/// Returns the byte length of a tightly packed mip-major volume.
pub fn volume_texture_size(format:u32,width:u32,height:u32,depth:u32,mips:u32)->Option<usize> {
    let (_,_,_,offset,len)=volume_mip_layout(format,width,height,depth,mips,mips.checked_sub(1)?)?;
    offset.checked_add(len)
}

#[derive(Clone, Debug)]
pub struct Batch { pub resources: Vec<Resource>, pub pipelines: Vec<Pipeline>, pub commands: Vec<Command> }

/// Whether a batch requires the array-capable host protocol.
pub fn requires_array_protocol(batch:&Batch)->bool {
    batch.resources.iter().any(|r|matches!(r.kind,ResourceKind::TextureArray {..}))
        || batch.commands.iter().any(|c|match c {
            Command::CopyTexture(c)=>c.slice!=0,
            Command::Draw(d)=>d.textures.iter().any(|t|t.array.is_some()),
            _=>false,
        })
}

/// Whether a batch contains a cube view requiring the cube-capable host protocol.
pub fn requires_cube_protocol(batch:&Batch)->bool {
    batch.commands.iter().any(|c|matches!(c,Command::Draw(d) if d.textures.iter().any(|t|t.cube)))
}

/// Whether a batch requires volume-capable transport.
pub fn requires_volume_protocol(batch:&Batch)->bool {
    batch.resources.iter().any(|r|matches!(r.kind,ResourceKind::Texture3D {..}))
        || batch.commands.iter().any(|c|match c {
            Command::Draw(d)=>d.textures.iter().any(|t|t.volume),
            Command::CopyTexture(c)=>c.depth!=1,
            _=>false,
        })
}
#[derive(Clone, Debug)]
pub struct Resource { pub id: u64, pub state: u32, pub kind: ResourceKind, pub bytes: Vec<u8> }
#[derive(Clone, Copy, Debug)]
pub enum ResourceKind { Buffer, Texture { width: u32, height: u32, format: u32, mips: u32 }, TextureArray { width:u32, height:u32, format:u32, mips:u32, layers:u32 }, Texture3D { width:u32, height:u32, depth:u32, format:u32, mips:u32 } }
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
    /// Additional contiguous render targets, indexed from one.
    pub extra_targets: Vec<u64>,
    pub vertex_buffer: u64, pub vertex_offset: u64, pub vertex_size: u32,
    pub vertex_stride:u32,
    pub vertex_count: u32, pub instance_count: u32, pub first_vertex: u32, pub first_instance: u32,
    pub viewport: [f32; 6], pub scissor: [i32; 4], pub topology: u32,
    pub constants: Vec<u32>, pub texture: Option<u64>, pub first_mip:u32, pub mip_count:u32,
    pub index: Option<Index>, pub depth: Option<u64>, pub linear_filter: bool,
    pub constant_slot:Option<u32>, pub texture_slot:Option<u32>, pub sampler_slot:Option<u32>,
    /// Explicit vertex views for the extended input-layout protocol; empty for legacy draws.
    pub vertices: Vec<VertexView>,
    /// Initialized entries keyed by root slot and table-relative descriptor index.
    pub textures: Vec<SampledTextureBinding>,
    pub samplers: Vec<SamplerBinding>,
}
#[derive(Clone, Debug)]
pub struct SampledTextureBinding { pub slot:u32, pub offset:u32, pub resource:u64, pub first_mip:u32, pub mip_count:u32, pub array:Option<(u32,u32)>, pub cube:bool, pub volume:bool }
#[derive(Clone, Debug)]
pub struct SamplerBinding { pub slot:u32, pub offset:u32, pub linear_filter:bool, pub address:[u32;3], pub mip_linear:bool, pub min_lod:f32, pub max_lod:f32 }
impl SamplerBinding {
    pub fn has_extended_lod(&self)->bool {
        self.mip_linear!=self.linear_filter || self.min_lod!=0.0 || self.max_lod!=0.0
    }
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
pub struct Pipeline { pub id:u64, pub vertex_shader:Vec<u8>, pub pixel_shader:Vec<u8>, pub vertex_stride:u32, pub format:u32, pub depth:Option<PipelineDepth>, pub blend:Blend, pub inputs:Vec<VertexInput>, pub extra_targets:Vec<ColorTarget>, pub cull:u32, pub front_ccw:bool }
#[derive(Clone, Debug)]
pub struct ColorTarget { pub format:u32, pub blend:Blend }
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
    pub width: u32, pub height: u32, pub depth:u32, pub format: u32, pub mip:u32, pub slice:u32, pub buffer_to_texture: bool,
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
        match r.kind { ResourceKind::Buffer => self.u32(0), ResourceKind::Texture {width,height,format,mips} => { self.u32(1); self.u32(width); self.u32(height); self.u32(format); self.u32(mips); }, ResourceKind::TextureArray {width,height,format,mips,layers} => { self.u32(2); for v in [width,height,format,mips,layers] {self.u32(v);} }, ResourceKind::Texture3D {width,height,depth,format,mips}=>{self.u32(3);for v in [width,height,depth,format,mips] {self.u32(v);}} }
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
    fn resource(&mut self)->Result<Resource,String> { let id=self.u64()?; let state=self.u32()?; let kind=match self.u32()? { 0=>ResourceKind::Buffer, 1=>ResourceKind::Texture {width:self.u32()?,height:self.u32()?,format:self.u32()?,mips:self.u32()?}, 2=>ResourceKind::TextureArray {width:self.u32()?,height:self.u32()?,format:self.u32()?,mips:self.u32()?,layers:self.u32()?}, 3=>ResourceKind::Texture3D {width:self.u32()?,height:self.u32()?,depth:self.u32()?,format:self.u32()?,mips:self.u32()?}, _=>return Err("unknown resource kind".into()) }; let resource=Resource {id,state,kind,bytes:self.bytes(MAX_RESOURCE)?}; validate_resource(&resource)?; Ok(resource) }
    fn done(&self)->Result<(),String> { if self.pos!=self.data.len() { Err("trailing wire data".into()) } else { Ok(()) } }
}
fn validate_resource(r:&Resource)->Result<(),String> {
    let expected=match r.kind {
        ResourceKind::Buffer=>return Ok(()),
        ResourceKind::Texture {width,height,format,mips}=>texture_size(format,width,height,mips),
        ResourceKind::TextureArray {width,height,format,mips,layers}=>array_texture_size(format,width,height,mips,layers),
        ResourceKind::Texture3D {width,height,depth,format,mips}=>volume_texture_size(format,width,height,depth,mips),
    };
    if expected!=Some(r.bytes.len()) {return Err("invalid texture format, dimensions, mip count or byte length".into())}
    Ok(())
}
fn validate_commands(resources:&[Resource],commands:&[Command])->Result<(),String> {
    for command in commands {
        match command {
            Command::Draw(d)=>{
                if d.extra_targets.len()>7 || d.target==0 {return Err("invalid render target count".into())}
                let mut targets=HashSet::new(); targets.insert(d.target);
                for &id in &d.extra_targets {
                    if id==0 || !targets.insert(id) || !matches!(resources.iter().find(|r|r.id==id).map(|r|r.kind),Some(ResourceKind::Texture {..})) {
                        return Err("invalid or duplicate render target".into());
                    }
                }
                let mut slots=HashSet::new();
                let mut texture_entries=HashSet::new();
                let mut sampler_entries=HashSet::new();
                let extended=!d.textures.is_empty() || !d.samplers.is_empty();
                if extended && (d.textures.len()>256 || d.samplers.len()>256 || d.texture.is_some()
                    || d.first_mip!=0 || d.mip_count!=0 || d.texture_slot.is_some()
                    || d.sampler_slot.is_some() || d.linear_filter)
                {return Err("invalid extended sampled bindings".into())}
                if !extended && d.texture.is_none() && (d.first_mip!=0 || d.mip_count!=0) {
                    return Err("texture view without texture".into());
                }
                for (slot,id,first,count,array,cube,volume) in d.textures.iter().map(|t|(Some((t.slot,t.offset)),t.resource,t.first_mip,t.mip_count,t.array,t.cube,t.volume))
                    .chain(d.texture.map(|id|(None,id,d.first_mip,d.mip_count,None,false,false)))
                {
                    if let Some((slot,offset))=slot {
                        if slot>=64 || offset>=256 || !texture_entries.insert((slot,offset)) {return Err("invalid texture slot or offset".into())}
                        slots.insert(slot);
                    }
                    if cube && (array!=Some((0,6)) || !matches!(resources.iter().find(|r|r.id==id).map(|r|r.kind),Some(ResourceKind::TextureArray {width,height,layers,..}) if width==height && layers>=6)) {
                        return Err("invalid cube texture view".into());
                    }
                    if volume && (array.is_some() || cube) {return Err("invalid volume texture view".into())}
                    let mips=match (resources.iter().find(|r|r.id==id).map(|r|r.kind),array,volume) {
                        (Some(ResourceKind::Texture {mips,..}),None,false)=>mips,
                        (Some(ResourceKind::Texture {mips,..}),Some((0,1)),false)=>mips,
                        (Some(ResourceKind::TextureArray {mips,layers,..}),Some((first_slice,slice_count)),false)
                            if slice_count!=0 && first_slice.checked_add(slice_count).is_some_and(|end|end<=layers)=>mips,
                        (Some(ResourceKind::Texture3D {mips,..}),None,true)=>mips,
                        _=>return Err("invalid sampled texture or array view".into()),
                    };
                    if count==0 || first.checked_add(count).is_none_or(|end|end>mips) {return Err("invalid texture view range".into())}
                }
                for sampler in &d.samplers {
                    if sampler.slot>=64 || sampler.offset>=256 || slots.contains(&sampler.slot) || !sampler_entries.insert((sampler.slot,sampler.offset)) {return Err("invalid sampler slot or offset".into())}
                    if sampler.address.iter().any(|v| !(1..=3).contains(v)) {return Err("invalid sampler address".into())}
                    if !sampler.min_lod.is_finite() || !sampler.max_lod.is_finite() || sampler.min_lod<0.0 || sampler.max_lod<sampler.min_lod {return Err("invalid sampler lod".into())}
                }
                slots.extend(d.samplers.iter().map(|s|s.slot));
                if extended && d.constant_slot.is_some_and(|slot| slots.contains(&slot)) {return Err("overlapping sampled and constant slots".into())}
            },
            Command::CopyTexture(c)=>{
                let id=if c.buffer_to_texture {c.dst} else {c.src};
                let (width,height,depth,format,mips,layers)=match resources.iter().find(|r|r.id==id).map(|r|r.kind) {
                    Some(ResourceKind::Texture {width,height,format,mips})=>(width,height,1,format,mips,1),
                    Some(ResourceKind::TextureArray {width,height,format,mips,layers})=>(width,height,1,format,mips,layers),
                    Some(ResourceKind::Texture3D {width,height,depth,format,mips})=>(width,height,depth,format,mips,1),
                    _=>return Err("unknown copy texture".into()),
                };
                if !matches!(resources.iter().find(|r|r.id==id).map(|r|r.kind),Some(ResourceKind::Texture3D {..})) {
                    if c.depth!=1 || c.slice>=layers || mip_layout(format,width,height,mips,c.mip).is_none() {return Err("invalid copy subresource".into())}
                } else {
                    let (w,h,d,_,_)=volume_mip_layout(format,width,height,depth,mips,c.mip).ok_or("invalid copy subresource")?;
                    if c.slice!=0 || c.width!=w || c.height!=h || c.depth!=d || c.format!=format {return Err("invalid copy dimensions or format".into())}
                    let (row,rows)=block_geometry(format,w,h).ok_or("invalid copy format")?;
                    let pitch=c.pitch as usize;
                    if pitch%256!=0 || pitch<row {return Err("invalid copy pitch".into())}
                    let len=rows.checked_mul(d as usize).and_then(|n|n.checked_sub(1)).and_then(|n|n.checked_mul(pitch)).and_then(|n|n.checked_add(row)).ok_or("copy size overflow")?;
                    let buffer_id=if c.buffer_to_texture {c.src} else {c.dst};
                    let buffer=resources.iter().find(|r|r.id==buffer_id && matches!(r.kind,ResourceKind::Buffer)).ok_or("unknown copy buffer")?;
                    if usize::try_from(c.offset).ok().and_then(|o|o.checked_add(len)).is_none_or(|end|end>buffer.bytes.len()) {return Err("copy buffer range out of bounds".into())}
                }
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
    let mut w=Writer(Vec::new()); resources(&mut w,&batch.resources)?;
    encode_metadata(&mut w,batch)?;
    finish(w)
}
fn encode_metadata(w:&mut Writer,batch:&Batch)->Result<(),String> {
    validate_commands(&batch.resources,&batch.commands)?;
    count(w,batch.pipelines.len())?;
    let mut ids=HashSet::new();
    for p in &batch.pipelines {
        if p.id==0 || !ids.insert(p.id) {return Err("duplicate pipeline id".into())}
        w.u64(p.id);w.bytes(&p.vertex_shader,MAX_SHADER)?;w.bytes(&p.pixel_shader,MAX_SHADER)?;
        if p.vertex_stride & 0x8000_0000 != 0 {return Err("invalid vertex stride".into())}
        if p.format & 0xc000_0000 != 0 || p.extra_targets.len()>7 {return Err("invalid render target formats".into())}
        if !(1..=3).contains(&p.cull) {return Err("invalid rasterizer cull mode".into())}
        let rasterizer=p.cull!=1 || p.front_ccw;
        w.u32(p.vertex_stride | if p.inputs.is_empty(){0}else{0x8000_0000});w.u32(p.format | if p.extra_targets.is_empty(){0}else{0x8000_0000} | if rasterizer {0x4000_0000}else{0});
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
        if !p.extra_targets.is_empty() {
            w.u32(p.extra_targets.len() as u32);
            for t in &p.extra_targets {
                if t.format & 0x8000_0000 != 0 || Blend::new(t.blend.enabled,t.blend.write_mask,t.blend.factors).is_none() {return Err("invalid render target state".into())}
                w.u32(t.format);w.u32(t.blend.enabled as u32);w.u32(t.blend.write_mask as u32);w.u32(t.blend.encode());
            }
        }
        if rasterizer {w.u32(p.cull);w.u32(p.front_ccw as u32);}
    }
    count(w,batch.commands.len())?;
    for c in &batch.commands { match c {
        Command::Clear {target,color} => {w.u32(0);w.u64(*target);for v in color {w.u32(v.to_bits());}},
        Command::Draw(d) => {let volume=d.textures.iter().any(|t|t.volume);let cube=volume || d.textures.iter().any(|t|t.cube);let arrays=cube || d.textures.iter().any(|t|t.array.is_some());let mrt=!d.extra_targets.is_empty();let lod=arrays || d.samplers.iter().any(SamplerBinding::has_extended_lod);let addressed=lod || d.samplers.iter().any(|s|s.address!=[3;3]);let extended=!d.textures.is_empty() || !d.samplers.is_empty();let offsets=mrt || addressed || d.textures.iter().any(|t|t.offset!=0) || d.samplers.iter().any(|s|s.offset!=0);w.u32(if volume {18} else if cube {17} else if arrays {16} else if lod {14} else if addressed {13} else if mrt {12} else if offsets {11} else if extended {10} else if d.vertices.is_empty(){1}else{9});w.u64(d.target);w.u64(d.pipeline);w.u64(d.vertex_buffer);w.u64(d.vertex_offset);for v in [d.vertex_size,d.vertex_count,d.instance_count,d.first_vertex,d.first_instance] {w.u32(v);} for v in d.viewport {w.u32(v.to_bits());} for v in d.scissor {w.u32(v as u32);} w.u32(d.topology); if d.constants.len()>64 {return Err("too many constants".into());} w.u32(d.constants.len() as u32);for v in &d.constants {w.u32(*v);} w.u64(d.texture.unwrap_or(0));w.u32(d.first_mip);w.u32(d.mip_count);
            if d.texture==Some(0) || d.depth==Some(0) || [d.constant_slot,d.texture_slot,d.sampler_slot].contains(&Some(u32::MAX)) {return Err("invalid optional identifier or slot".into())}
            if let Some(i)=&d.index {w.u32(1);w.u64(i.buffer);w.u64(i.offset);for v in [i.size,i.format,i.first,i.base_vertex as u32] {w.u32(v);}} else {w.u32(0);}
            w.u64(d.depth.unwrap_or(0));w.u32(d.linear_filter as u32);
            w.u32(d.vertex_stride);for slot in [d.constant_slot,d.texture_slot,d.sampler_slot] {w.u32(slot.unwrap_or(u32::MAX));}
            if !d.vertices.is_empty() || extended || mrt {
                if d.vertices.len()>16 {return Err("too many vertex views".into())}
                w.u32(d.vertices.len() as u32);
                let mut slots=0u32;
                for v in &d.vertices {
                    if v.slot>=16 || v.buffer==0 || v.size==0 || v.stride==0 || slots & (1<<v.slot)!=0 {return Err("invalid vertex view".into())}
                    slots|=1<<v.slot;w.u32(v.slot);w.u64(v.buffer);w.u64(v.offset);w.u32(v.size);w.u32(v.stride);
                }
            }
            if extended || mrt {
                w.u32(d.textures.len() as u32);
                for t in &d.textures {w.u32(t.slot);if offsets {w.u32(t.offset);}w.u64(t.resource);w.u32(t.first_mip);w.u32(t.mip_count);if arrays {let (first,count)=t.array.unwrap_or((0,0));w.u32(first);w.u32(count);}if cube {w.u32(t.cube as u32);}if volume {w.u32(t.volume as u32);}}
                w.u32(d.samplers.len() as u32);
                for s in &d.samplers {w.u32(s.slot);if offsets {w.u32(s.offset);}w.u32(if addressed {(s.linear_filter as u32) | (s.address[0]<<1) | (s.address[1]<<3) | (s.address[2]<<5)} else {s.linear_filter as u32});if lod {w.u32(s.mip_linear as u32);w.u32(s.min_lod.to_bits());w.u32(s.max_lod.to_bits());}}
            }
            if mrt || addressed {w.u32(d.extra_targets.len() as u32);for &id in &d.extra_targets {w.u64(id);}}
        },
        Command::Barrier {id,before,after} => {w.u32(2);w.u64(*id);w.u32(*before);w.u32(*after);},
        Command::CopyTexture(c) => {let volume=matches!(batch.resources.iter().find(|r|r.id==if c.buffer_to_texture {c.dst}else{c.src}).map(|r|r.kind),Some(ResourceKind::Texture3D {..}));w.u32(if volume {19} else if c.slice==0 {3}else{15});w.u64(c.src);w.u64(c.dst);w.u64(c.offset);for v in [c.pitch,c.width,c.height,c.format,c.mip,c.buffer_to_texture as u32] {w.u32(v);}if volume || c.slice!=0 {w.u32(c.slice);}if volume {w.u32(c.depth);}},
        Command::CopyBuffer {src,dst,src_offset,dst_offset,size} => {w.u32(4);for v in [src,dst,src_offset,dst_offset,size] {w.u64(*v);}},
        Command::ClearDepth {target,depth}=>{w.u32(5);w.u64(*target);w.u32(depth.to_bits());},
        Command::SetStencilRef(value)=>{w.u32(6);w.u32(*value);},
        Command::ClearStencil {target,flags,depth,stencil}=>{w.u32(7);w.u64(*target);w.u32(*flags);w.u32(depth.to_bits());w.u32(u32::from(*stencil));},
        Command::CopyDepthStencil {src,dst,offset,pitch,width,height,plane,upload}=>{w.u32(8);w.u64(*src);w.u64(*dst);w.u64(*offset);for v in [*pitch,*width,*height,*plane,*upload as u32] {w.u32(v);}},
    }} Ok(())
}
pub fn decode_batch(data:&[u8])->Result<Batch,String> {
    if data.len()>MAX_PAYLOAD {return Err("wire payload exceeds limit".into());} let mut r=Reader {data,pos:0}; let resources=read_resources(&mut r)?;
    let batch=decode_metadata(&mut r,resources)?; r.done()?; Ok(batch)
}
fn decode_metadata(r:&mut Reader<'_>,resources:Vec<Resource>)->Result<Batch,String> {
    let n=r.count()?;let mut pipelines=Vec::with_capacity(n);let mut ids=HashSet::new();
    for _ in 0..n {
        let id=r.u64()?;if id==0 || !ids.insert(id) {return Err("duplicate pipeline id".into())}
        let vertex_shader=r.bytes(MAX_SHADER)?;let pixel_shader=r.bytes(MAX_SHADER)?;
        let raw_stride=r.u32()?;let extended=raw_stride & 0x8000_0000!=0;let vertex_stride=raw_stride & 0x7fff_ffff;
        let raw_format=r.u32()?;let mrt=raw_format & 0x8000_0000!=0;let rasterizer=raw_format & 0x4000_0000!=0;let format=raw_format & 0x3fff_ffff;
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
        let mut extra_targets=Vec::new();
        if mrt {
            let n=r.count()?;if n==0 || n>7 {return Err("invalid render target count".into())}
            for _ in 0..n {
                let format=r.u32()?;let enabled=r.boolean()?;let mask=r.u32()?;
                if format & 0x8000_0000!=0 || mask>15 {return Err("invalid render target state".into())}
                let blend=Blend::decode(enabled,mask as u8,r.u32()?).ok_or("invalid render target blend")?;
                extra_targets.push(ColorTarget {format,blend});
            }
        }
        let (cull,front_ccw)=if rasterizer {
            let cull=r.u32()?;if !(1..=3).contains(&cull) {return Err("invalid rasterizer cull mode".into())}
            (cull,r.boolean()?)
        } else {(1,false)};
        pipelines.push(Pipeline {id,vertex_shader,pixel_shader,vertex_stride,format,depth,blend,inputs,extra_targets,cull,front_ccw});
    }
    let n=r.count()?; let mut commands=Vec::with_capacity(n);
    for _ in 0..n { commands.push(match r.u32()? {
        0=> {let target=r.u64()?;let mut color=[0.;4];for v in &mut color {*v=f32::from_bits(r.u32()?);} Command::Clear {target,color}},
        kind @ (1|9|10|11|12|13|14|16|17|18)=> {let target=r.u64()?;let pipeline=r.u64()?;let vertex_buffer=r.u64()?;let vertex_offset=r.u64()?;let vertex_size=r.u32()?;let vertex_count=r.u32()?;let instance_count=r.u32()?;let first_vertex=r.u32()?;let first_instance=r.u32()?;let mut viewport=[0.;6];for v in &mut viewport {*v=f32::from_bits(r.u32()?);} let mut scissor=[0;4];for v in &mut scissor {*v=r.u32()? as i32;} let topology=r.u32()?;let count=r.u32()? as usize;if count>64 {return Err("too many constants".into());} let mut constants=Vec::with_capacity(count);for _ in 0..count {constants.push(r.u32()?);} let texture=match r.u64()? {0=>None,id=>Some(id)};let first_mip=r.u32()?;let mip_count=r.u32()?;
            let index=if r.boolean()? {Some(Index {buffer:r.u64()?,offset:r.u64()?,size:r.u32()?,format:r.u32()?,first:r.u32()?,base_vertex:r.u32()? as i32})} else {None};
            let depth=match r.u64()? {0=>None,id=>Some(id)};let linear_filter=r.boolean()?;let vertex_stride=r.u32()?;let mut slot=|| -> Result<Option<u32>,String> {Ok(match r.u32()? {u32::MAX=>None,v=>Some(v)})};let constant_slot=slot()?;let texture_slot=slot()?;let sampler_slot=slot()?;
            let mut vertices=Vec::new();
            if kind>=9 {
                let n=r.count()?;if (kind==9 && n==0) || n>16 {return Err("invalid vertex view count".into())}
                let mut slots=0u32;
                for _ in 0..n {
                    let v=VertexView {slot:r.u32()?,buffer:r.u64()?,offset:r.u64()?,size:r.u32()?,stride:r.u32()?};
                    if v.slot>=16 || v.buffer==0 || v.size==0 || v.stride==0 || slots & (1<<v.slot)!=0 {return Err("invalid vertex view".into())}
                    slots|=1<<v.slot;vertices.push(v);
                }
            }
            let mut textures=Vec::new();let mut samplers=Vec::new();
            if kind>=10 {
                let n=r.count()?;if n>256 {return Err("invalid texture binding count".into())}
                for _ in 0..n {
                    let slot=r.u32()?;let offset=if kind>=11 {r.u32()?} else {0};let resource=r.u64()?;let first_mip=r.u32()?;let mip_count=r.u32()?;
                    let array=if kind>=16 {let first=r.u32()?;let count=r.u32()?;if count==0 {if first!=0 {return Err("invalid array view".into())} None} else {Some((first,count))}} else {None};
                    let cube=if kind>=17 {r.boolean()?} else {false};
                    let volume=if kind==18 {r.boolean()?} else {false};
                    textures.push(SampledTextureBinding {slot,offset,resource,first_mip,mip_count,array,cube,volume});
                }
                let n=r.count()?;if n>256 || (kind!=12 && n==0 && textures.is_empty()) {return Err("invalid sampler binding count".into())}
                for _ in 0..n {
                    let slot=r.u32()?;let offset=if kind>=11 {r.u32()?} else {0};
                    let (linear_filter,address)=if kind>=13 {
                        let packed=r.u32()?;
                        let address=[(packed>>1)&3,(packed>>3)&3,(packed>>5)&3];
                        if packed & !0x7f != 0 || address.contains(&0) {return Err("invalid packed sampler".into())}
                        (packed & 1 != 0,address)
                    } else {(r.boolean()?,[3;3])};
                    let (mip_linear,min_lod,max_lod)=if kind==14 || kind>=16 {
                        (r.boolean()?,f32::from_bits(r.u32()?),f32::from_bits(r.u32()?))
                    } else {(linear_filter,0.0,0.0)};
                    samplers.push(SamplerBinding {slot,offset,linear_filter,address,mip_linear,min_lod,max_lod});
                }
            }
            let mut extra_targets=Vec::new();
            if kind>=12 {let n=r.count()?;if (kind==12 && n==0) || n>7 {return Err("invalid render target count".into())}for _ in 0..n {extra_targets.push(r.u64()?);}}
            Command::Draw(Draw {target,extra_targets,pipeline,vertex_buffer,vertex_offset,vertex_size,vertex_stride,vertex_count,instance_count,first_vertex,first_instance,viewport,scissor,topology,constants,texture,first_mip,mip_count,index,depth,linear_filter,constant_slot,texture_slot,sampler_slot,vertices,textures,samplers})},
        2=>Command::Barrier {id:r.u64()?,before:r.u32()?,after:r.u32()?},
        kind @ (3|15|19)=> {let src=r.u64()?;let dst=r.u64()?;let offset=r.u64()?;let pitch=r.u32()?;let width=r.u32()?;let height=r.u32()?;let format=r.u32()?;let mip=r.u32()?;let buffer_to_texture=match r.u32()? {0=>false,1=>true,_=>return Err("invalid copy direction".into())};let slice=if kind==15 || kind==19 {r.u32()?} else {0};let depth=if kind==19 {r.u32()?}else{1};Command::CopyTexture(CopyTexture {src,dst,offset,pitch,width,height,depth,format,mip,slice,buffer_to_texture})},
        4=>Command::CopyBuffer {src:r.u64()?,dst:r.u64()?,src_offset:r.u64()?,dst_offset:r.u64()?,size:r.u64()?},
        5=>Command::ClearDepth {target:r.u64()?,depth:f32::from_bits(r.u32()?)},
        6=>Command::SetStencilRef(r.u32()?),
        7=>Command::ClearStencil {target:r.u64()?,flags:r.u32()?,depth:f32::from_bits(r.u32()?),stencil:u8::try_from(r.u32()?).map_err(|_|"invalid stencil clear")?},
        8=>Command::CopyDepthStencil {src:r.u64()?,dst:r.u64()?,offset:r.u64()?,pitch:r.u32()?,width:r.u32()?,height:r.u32()?,plane:r.u32()?,upload:r.boolean()?},
        _=>return Err("unknown wire command".into()),
    }); } validate_commands(&resources,&commands)?;Ok(Batch {resources,pipelines,commands})
}

// Version 9 consists of a resource table followed, for batches, by a
// length-prefixed version-5 pipeline/command section. Resource contents occur
// only once; the batch limit applies to their combined byte lengths.
const STREAM_CHUNK: usize = 64 * 1024;

fn stream_add(total:&mut usize,n:usize)->Result<(),String> {
    *total=total.checked_add(n).filter(|&v|v<=MAX_STREAM_BATCH)
        .ok_or("stream batch exceeds limit")?;
    Ok(())
}

fn stream_resource_size(rs:&[Resource])->Result<usize,String> {
    if rs.len()>MAX_ITEMS {return Err("too many wire items".into())}
    let mut total=0;
    let mut ids=HashSet::new();
    for r in rs {
        if r.id==0 || !ids.insert(r.id) {return Err("duplicate or zero resource id".into())}
        if r.bytes.len()>MAX_STREAM_RESOURCE {return Err("stream resource exceeds limit".into())}
        validate_resource(r)?;
        stream_add(&mut total,r.bytes.len())?;
    }
    Ok(total)
}

fn stream_write_resources<W:std::io::Write>(w:&mut W,rs:&[Resource])->Result<(),String> {
    fn word<W:std::io::Write>(w:&mut W,n:u32)->Result<(),String> {
        w.write_all(&n.to_le_bytes()).map_err(|e|e.to_string())
    }
    word(w,rs.len() as u32)?;
    for r in rs {
        w.write_all(&r.id.to_le_bytes()).map_err(|e|e.to_string())?;
        word(w,r.state)?;
        match r.kind {
            ResourceKind::Buffer=>word(w,0)?,
            ResourceKind::Texture {width,height,format,mips}=>{
                word(w,1)?;
                for v in [width,height,format,mips] {word(w,v)?;}
            }
            ResourceKind::TextureArray {width,height,format,mips,layers}=>{
                word(w,2)?;
                for v in [width,height,format,mips,layers] {word(w,v)?;}
            }
            ResourceKind::Texture3D {width,height,depth,format,mips}=>{
                word(w,3)?;
                for v in [width,height,depth,format,mips] {word(w,v)?;}
            }
        }
        word(w,r.bytes.len() as u32)?;
        for chunk in r.bytes.chunks(STREAM_CHUNK) {
            w.write_all(chunk).map_err(|e|e.to_string())?;
        }
    }
    Ok(())
}

struct StreamReader<'a,R> { input:&'a mut R, total:usize }
impl<R:std::io::Read> StreamReader<'_,R> {
    fn read(&mut self,bytes:&mut [u8])->Result<(),String> {
        self.input.read_exact(bytes).map_err(|e|format!("stream read failed: {e}"))
    }
    fn u32(&mut self)->Result<u32,String> {
        let mut b=[0;4]; self.read(&mut b)?; Ok(u32::from_le_bytes(b))
    }
    fn u64(&mut self)->Result<u64,String> {
        let mut b=[0;8]; self.read(&mut b)?; Ok(u64::from_le_bytes(b))
    }
    fn resources(&mut self)->Result<Vec<Resource>,String> {
        let n=self.u32()? as usize;
        if n>MAX_ITEMS {return Err("too many wire items".into())}
        let mut rs=Vec::with_capacity(n);
        let mut ids=HashSet::new();
        for _ in 0..n {
            let id=self.u64()?;
            if id==0 || !ids.insert(id) {return Err("duplicate or zero resource id".into())}
            let state=self.u32()?;
            let kind=match self.u32()? {
                0=>ResourceKind::Buffer,
                1=>ResourceKind::Texture {width:self.u32()?,height:self.u32()?,format:self.u32()?,mips:self.u32()?},
                2=>ResourceKind::TextureArray {width:self.u32()?,height:self.u32()?,format:self.u32()?,mips:self.u32()?,layers:self.u32()?},
                3=>ResourceKind::Texture3D {width:self.u32()?,height:self.u32()?,depth:self.u32()?,format:self.u32()?,mips:self.u32()?},
                _=>return Err("unknown resource kind".into()),
            };
            let len=self.u32()? as usize;
            if len>MAX_STREAM_RESOURCE {return Err("stream resource exceeds limit".into())}
            let expected=match kind {
                ResourceKind::Buffer=>Some(len),
                ResourceKind::Texture {width,height,format,mips}=>texture_size(format,width,height,mips),
                ResourceKind::TextureArray {width,height,format,mips,layers}=>array_texture_size(format,width,height,mips,layers),
                ResourceKind::Texture3D {width,height,depth,format,mips}=>volume_texture_size(format,width,height,depth,mips),
            };
            if expected!=Some(len) {return Err("invalid texture format, dimensions, mip count or byte length".into());}
            // Check the whole allocation against the remaining budget before allocating.
            stream_add(&mut self.total,len)?;
            let mut bytes=vec![0;len];
            for chunk in bytes.chunks_mut(STREAM_CHUNK) {self.read(chunk)?;}
            rs.push(Resource {id,state,kind,bytes});
        }
        Ok(rs)
    }
}

/// Validates a streamed result before committing a response header.
pub fn validate_stream_results(rs:&[Resource])->Result<(),String> {
    stream_resource_size(rs).map(|_| ())
}

pub fn write_stream_results<W:std::io::Write>(w:&mut W,rs:&[Resource])->Result<(),String> {
    validate_stream_results(rs)?;
    stream_write_resources(w,rs)
}

pub fn read_stream_results<R:std::io::Read>(r:&mut R)->Result<Vec<Resource>,String> {
    StreamReader {input:r,total:0}.resources()
}

pub fn write_stream_batch<W:std::io::Write>(w:&mut W,batch:&Batch)->Result<(),String> {
    stream_resource_size(&batch.resources)?;
    let mut metadata=Writer(Vec::new());
    encode_metadata(&mut metadata,batch)?;
    let metadata=finish(metadata)?;
    stream_write_resources(w,&batch.resources)?;
    w.write_all(&(metadata.len() as u32).to_le_bytes()).map_err(|e|e.to_string())?;
    for chunk in metadata.chunks(STREAM_CHUNK) {w.write_all(chunk).map_err(|e|e.to_string())?;}
    Ok(())
}

pub fn read_stream_batch<R:std::io::Read>(r:&mut R)->Result<Batch,String> {
    let mut stream=StreamReader {input:r,total:0};
    let resources=stream.resources()?;
    let len=stream.u32()? as usize;
    if len>MAX_PAYLOAD {
        return Err("stream metadata exceeds limit".into());
    }
    let mut metadata=vec![0;len];
    for chunk in metadata.chunks_mut(STREAM_CHUNK) {stream.read(chunk)?;}
    let mut reader=Reader {data:&metadata,pos:0};
    let batch=decode_metadata(&mut reader,resources)?;
    reader.done()?;
    Ok(batch)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn volume_layout_and_wire_round_trip() {
        assert_eq!(volume_mip_layout(28,4,2,4,3,0),Some((4,2,4,0,128)));
        assert_eq!(volume_mip_layout(28,4,2,4,3,1),Some((2,1,2,128,16)));
        assert_eq!(volume_mip_layout(28,4,2,4,3,2),Some((1,1,1,144,4)));
        assert_eq!(volume_texture_size(28,4,2,4,3),Some(148));
        assert_eq!(volume_texture_size(28,4,2,0,1),None);
        assert_eq!(volume_texture_size(28,4,2,4,4),None);
        assert_eq!(volume_texture_size(28,u32::MAX,u32::MAX,u32::MAX,1),None);
        assert_eq!(volume_mip_layout(71,4,4,2,1,0),None);

        let mut b=batch();
        b.resources[0].bytes=vec![0;1808];
        b.resources.push(Resource {id:3,state:0,kind:ResourceKind::Texture3D {width:4,height:2,depth:4,format:28,mips:3},bytes:vec![0;148]});
        b.commands=vec![Command::CopyTexture(CopyTexture {src:1,dst:3,offset:0,pitch:256,width:4,height:2,depth:4,format:28,mip:0,slice:0,buffer_to_texture:true}),
            Command::Draw(Draw {target:2,extra_targets:vec![],pipeline:7,vertex_buffer:1,vertex_offset:0,vertex_size:4,vertex_stride:16,
                vertex_count:3,instance_count:1,first_vertex:0,first_instance:0,viewport:[0.,0.,1.,1.,0.,1.],scissor:[0,0,1,1],
                topology:4,constants:vec![],texture:None,first_mip:0,mip_count:0,index:None,depth:None,linear_filter:false,
                constant_slot:None,texture_slot:None,sampler_slot:None,vertices:vec![],
                textures:vec![SampledTextureBinding {slot:1,offset:0,resource:3,first_mip:1,mip_count:2,array:None,cube:false,volume:true}],samplers:vec![]})];
        assert!(requires_volume_protocol(&b));
        let encoded=encode_batch(&b).unwrap();
        let opcode=encode_results(&b.resources).unwrap().len()+8;
        assert_eq!(&encoded[opcode..opcode+4],&19u32.to_le_bytes());
        assert_eq!(&encoded[opcode+60..opcode+64],&18u32.to_le_bytes());
        let decoded=decode_batch(&encoded).unwrap();
        assert_eq!(encode_batch(&decoded).unwrap(),encoded);
        let mut streamed=Vec::new();
        write_stream_batch(&mut streamed,&b).unwrap();
        let decoded=read_stream_batch(&mut streamed.as_slice()).unwrap();
        assert!(matches!(&decoded.commands[1],Command::Draw(d) if d.textures[0].volume));
        for end in 0..encoded.len() {assert!(decode_batch(&encoded[..end]).is_err());}
        if let Command::CopyTexture(c)=&mut b.commands[0] {c.depth=3;}
        assert!(encode_batch(&b).is_err());
        if let Command::CopyTexture(c)=&mut b.commands[0] {c.depth=4;c.offset=1;}
        assert!(encode_batch(&b).is_err());
        if let Command::CopyTexture(c)=&mut b.commands[0] {c.offset=0;}
        if let Command::Draw(d)=&mut b.commands[1] {d.textures[0].array=Some((0,1));}
        assert!(encode_batch(&b).is_err());
    }

    #[test]
    fn cube_views_round_trip_and_validate() {
        let mut b=batch();
        b.resources.push(Resource {id:3,state:0,kind:ResourceKind::TextureArray {width:4,height:4,format:28,mips:2,layers:6},bytes:vec![0;480]});
        b.commands=vec![Command::Draw(Draw {target:2,extra_targets:vec![],pipeline:7,vertex_buffer:1,vertex_offset:0,vertex_size:4,vertex_stride:16,
            vertex_count:3,instance_count:1,first_vertex:0,first_instance:0,viewport:[0.,0.,1.,1.,0.,1.],scissor:[0,0,1,1],
            topology:4,constants:vec![],texture:None,first_mip:0,mip_count:0,index:None,depth:None,linear_filter:false,
            constant_slot:None,texture_slot:None,sampler_slot:None,vertices:vec![],
            textures:vec![SampledTextureBinding {slot:1,offset:0,resource:3,first_mip:0,mip_count:2,array:Some((0,6)),cube:true,volume:false},
                SampledTextureBinding {slot:1,offset:1,resource:2,first_mip:0,mip_count:1,array:None,cube:false,volume:false}],
            samplers:vec![SamplerBinding {slot:2,offset:0,linear_filter:true,address:[1,2,3],mip_linear:true,min_lod:0.,max_lod:1.}]})];
        assert!(requires_cube_protocol(&b));
        let bytes=encode_batch(&b).unwrap();
        let opcode=encode_results(&b.resources).unwrap().len()+8;
        assert_eq!(&bytes[opcode..opcode+4],&17u32.to_le_bytes());
        let decoded=decode_batch(&bytes).unwrap();
        let Command::Draw(d)=&decoded.commands[0] else {unreachable!()};
        assert!(d.textures[0].cube);
        assert!(!d.textures[1].cube);
        assert_eq!(encode_batch(&decoded).unwrap(),bytes);
        let mut stream=Vec::new();
        write_stream_batch(&mut stream,&b).unwrap();
        assert_eq!(encode_batch(&read_stream_batch(&mut stream.as_slice()).unwrap()).unwrap(),bytes);
        for end in 0..bytes.len() {assert!(decode_batch(&bytes[..end]).is_err());}
        let cube_flag=bytes.len()-4-24-4-36-4;
        let mut malformed=bytes.clone();
        malformed[cube_flag..cube_flag+4].copy_from_slice(&2u32.to_le_bytes());
        assert!(decode_batch(&malformed).is_err());
        let Command::Draw(d)=&mut b.commands[0] else {unreachable!()};
        d.textures[0].array=Some((1,5));
        assert!(encode_batch(&b).is_err());
        let Command::Draw(d)=&mut b.commands[0] else {unreachable!()};
        d.textures[0].array=None;
        assert!(encode_batch(&b).is_err());
        let Command::Draw(d)=&mut b.commands[0] else {unreachable!()};
        d.textures[0].array=Some((0,6));
        b.resources[2].kind=ResourceKind::TextureArray {width:4,height:2,format:28,mips:2,layers:6};
        b.resources[2].bytes=vec![0;240];
        assert!(encode_batch(&b).is_err());
        b.resources[2].kind=ResourceKind::TextureArray {width:4,height:4,format:28,mips:2,layers:5};
        b.resources[2].bytes=vec![0;400];
        assert!(encode_batch(&b).is_err());
    }

    #[test]
    fn array_layout_and_wire_round_trip() {
        assert_eq!(array_texture_size(28,4,4,2,3),Some(240));
        assert_eq!(array_mip_layout(28,4,4,2,3,1,1),Some((2,2,144,16)));
        assert_eq!(array_texture_size(71,7,5,2,2),Some(80));
        assert_eq!(array_mip_layout(71,7,5,2,2,1,1),Some((3,2,72,8)));
        assert_eq!(array_texture_size(28,4,4,2,0),None);
        assert_eq!(array_texture_size(20,4,4,1,2),None);
        assert_eq!(array_mip_layout(28,4,4,2,3,3,0),None);
        assert_eq!(array_mip_layout(28,4,4,2,3,0,2),None);

        let mut b=batch();
        b.resources.push(Resource {id:3,state:0,kind:ResourceKind::TextureArray {width:4,height:4,format:28,mips:2,layers:3},bytes:vec![9;240]});
        b.commands=vec![
            Command::CopyTexture(CopyTexture {src:1,dst:3,offset:0,pitch:8,width:2,height:2,depth:1,format:28,mip:1,slice:2,buffer_to_texture:true}),
            Command::Draw(Draw {target:2,extra_targets:vec![],pipeline:7,vertex_buffer:1,vertex_offset:0,vertex_size:4,vertex_stride:16,
                vertex_count:3,instance_count:1,first_vertex:0,first_instance:0,viewport:[0.,0.,1.,1.,0.,1.],scissor:[0,0,1,1],
                topology:4,constants:vec![],texture:None,first_mip:0,mip_count:0,index:None,depth:None,linear_filter:false,
                constant_slot:None,texture_slot:None,sampler_slot:None,vertices:vec![],
                textures:vec![SampledTextureBinding {slot:1,offset:0,resource:3,first_mip:1,mip_count:1,array:Some((1,2)),cube:false,volume:false},
                    SampledTextureBinding {slot:1,offset:1,resource:2,first_mip:0,mip_count:1,array:None,cube:false,volume:false}],samplers:vec![]}),
        ];
        assert!(requires_array_protocol(&b));
        let bytes=encode_batch(&b).unwrap();
        let decoded=decode_batch(&bytes).unwrap();
        assert_eq!(encode_batch(&decoded).unwrap(),bytes);
        assert!(matches!(&decoded.commands[0],Command::CopyTexture(CopyTexture {slice:2,mip:1,..})));
        if let Command::Draw(d)=&decoded.commands[1] {
            assert_eq!(d.textures[0].array,Some((1,2)));
            assert_eq!(d.textures[1].array,None);
        } else {panic!("expected draw")}
        let mut stream=Vec::new();
        write_stream_batch(&mut stream,&b).unwrap();
        let streamed=read_stream_batch(&mut &stream[..]).unwrap();
        assert_eq!(encode_batch(&streamed).unwrap(),bytes);
        let mut results=Vec::new();
        write_stream_results(&mut results,&b.resources).unwrap();
        assert_eq!(encode_results(&read_stream_results(&mut &results[..]).unwrap()).unwrap(),encode_results(&b.resources).unwrap());
        if let Command::Draw(d)=&mut b.commands[1] {d.textures[0].array=Some((2,2));}
        assert!(encode_batch(&b).is_err());
        if let Command::Draw(d)=&mut b.commands[1] {d.textures[0].array=None;}
        assert!(encode_batch(&b).is_err());
        if let Command::Draw(d)=&mut b.commands[1] {d.textures[0].array=Some((1,2));}
        if let Command::CopyTexture(c)=&mut b.commands[0] {c.slice=3;}
        assert!(encode_batch(&b).is_err());
        b.resources[2].bytes.pop();
        assert!(encode_results(&b.resources).is_err());
    }


    #[cfg(unix)]
    #[test]
    fn stream_large_batch_and_results() {
        use std::os::unix::net::UnixStream;
        let mut b=Batch {resources:Vec::new(),pipelines:vec![],commands:vec![]};
        for id in 1..=4 {
            b.resources.push(Resource {id,state:id as u32,kind:ResourceKind::Buffer,
                bytes:vec![id as u8;17*1024*1024]});
        }
        b.commands.push(Command::CopyBuffer {src:1,dst:2,src_offset:0,dst_offset:0,size:8});
        let (mut tx,mut rx)=UnixStream::pair().unwrap();
        let decoded=std::thread::scope(|s| {
            let writer=s.spawn(||write_stream_batch(&mut tx,&b).unwrap());
            let decoded=read_stream_batch(&mut rx).unwrap();
            writer.join().unwrap();
            decoded
        });
        assert_eq!(decoded.resources.len(),4);
        assert_eq!(decoded.resources[3].bytes.len(),17*1024*1024);
        assert_eq!(decoded.resources[3].bytes[17*1024*1024-1],4);
        assert!(matches!(decoded.commands[0],Command::CopyBuffer {src:1,dst:2,..}));
        drop(b);
        let (mut tx,mut rx)=UnixStream::pair().unwrap();
        std::thread::scope(|s| {
            let writer=s.spawn(||write_stream_results(&mut tx,&decoded.resources).unwrap());
            let results=read_stream_results(&mut rx).unwrap();
            writer.join().unwrap();
            assert_eq!(results[0].bytes.len(),17*1024*1024);
            assert_eq!(results[2].bytes[0],3);
        });
    }

    #[test]
    fn stream_rejects_malformed_and_truncated() {
        let b=batch();
        let mut wire=Vec::new();
        write_stream_batch(&mut wire,&b).unwrap();
        for end in 0..wire.len() {
            assert!(read_stream_batch(&mut &wire[..end]).is_err(),"truncation at {end}");
        }
        let decoded=read_stream_batch(&mut &wire[..]).unwrap();
        assert_eq!(decoded.resources[1].bytes,b.resources[1].bytes);
        let mut invalid_metadata=wire.clone();
        let metadata_start=4+20+4+36+4+4;
        invalid_metadata[metadata_start+8..metadata_start+12].copy_from_slice(&99u32.to_le_bytes());
        assert!(read_stream_batch(&mut &invalid_metadata[..]).is_err());
        let mut result=Vec::new();
        write_stream_results(&mut result,&b.resources).unwrap();
        for end in 0..result.len() {
            assert!(read_stream_results(&mut &result[..end]).is_err(),"truncation at {end}");
        }
        let mut duplicate=result.clone();
        duplicate[28..36].copy_from_slice(&1u64.to_le_bytes());
        assert!(read_stream_results(&mut &duplicate[..]).is_err());
        let mut unknown=result.clone();
        unknown[16..20].copy_from_slice(&2u32.to_le_bytes());
        assert!(read_stream_results(&mut &unknown[..]).is_err());
        let mut bad_texture=result.clone();
        bad_texture[52..56].copy_from_slice(&70u32.to_le_bytes());
        assert!(read_stream_results(&mut &bad_texture[..]).is_err());
        let mut duplicate_batch=batch();
        duplicate_batch.resources[1].id=1;
        assert!(write_stream_batch(&mut Vec::new(),&duplicate_batch).is_err());
        let mut bad_command=batch();
        bad_command.commands.push(Command::CopyTexture(CopyTexture {src:1,dst:2,offset:0,pitch:4,width:1,height:1,depth:1,format:28,mip:1,slice:0,buffer_to_texture:true}));
        assert!(write_stream_batch(&mut Vec::new(),&bad_command).is_err());
    }

    #[test]
    fn stream_limits_checked_before_allocation() {
        let mut total=MAX_STREAM_BATCH;
        assert!(stream_add(&mut total,1).is_err());
        let mut total=usize::MAX;
        assert!(stream_add(&mut total,1).is_err());
        let mut oversized=Vec::new();
        oversized.extend_from_slice(&1u32.to_le_bytes());
        oversized.extend_from_slice(&1u64.to_le_bytes());
        oversized.extend_from_slice(&0u32.to_le_bytes());
        oversized.extend_from_slice(&0u32.to_le_bytes());
        oversized.extend_from_slice(&((MAX_STREAM_RESOURCE+1) as u32).to_le_bytes());
        assert!(read_stream_results(&mut &oversized[..]).is_err());
        let mut aggregate=oversized.clone();
        aggregate[20..24].copy_from_slice(&2u32.to_le_bytes());
        let mut input=&aggregate[..];
        let mut reader=StreamReader {input:&mut input,total:MAX_STREAM_BATCH-1};
        assert!(reader.resources().unwrap_err().contains("batch exceeds"));
        let mut count=Vec::new();
        count.extend_from_slice(&((MAX_ITEMS+1) as u32).to_le_bytes());
        assert!(read_stream_results(&mut &count[..]).is_err());
        let mut bad_metadata=Vec::new();
        bad_metadata.extend_from_slice(&0u32.to_le_bytes());
        bad_metadata.extend_from_slice(&((MAX_PAYLOAD+1) as u32).to_le_bytes());
        assert!(read_stream_batch(&mut &bad_metadata[..]).is_err());
    }

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
            depth:Some(PipelineDepth {write:true,compare:4,stencil:None}),blend:Blend::new(true,15,[3,7,2,5,8,3]).unwrap(),inputs:vec![],extra_targets:vec![],cull:1,front_ccw:false});
        b.commands.push(Command::Draw(Draw {target:2,extra_targets:vec![],pipeline:7,vertex_buffer:1,vertex_offset:0,vertex_size:4,vertex_stride:16,
            vertex_count:3,instance_count:1,first_vertex:0,first_instance:0,viewport:[0.,0.,1.,1.,0.,1.],scissor:[0,0,1,1],
            topology:4,constants:vec![10,20],texture:Some(2),first_mip:0,mip_count:1,index:Some(Index {buffer:1,offset:0,size:4,format:57,first:0,base_vertex:-1}),
            depth:Some(3),linear_filter:true,constant_slot:Some(3),texture_slot:Some(2),sampler_slot:Some(1),vertices:vec![],textures:vec![],samplers:vec![]}));
        let encoded=encode_batch(&b).unwrap();
        assert_eq!(encode_batch(&decode_batch(&encoded).unwrap()).unwrap(),encoded);
        b.pipelines.push(b.pipelines[0].clone());
        assert!(encode_batch(&b).is_err());
    }

    #[test]
    fn rasterizer_wire_legacy_extended_and_invalid() {
        let mut b=Batch {resources:vec![],pipelines:vec![Pipeline {
            id:1,vertex_shader:vec![],pixel_shader:vec![],vertex_stride:16,format:28,
            depth:None,blend:Blend::new(false,15,[0;6]).unwrap(),inputs:vec![],extra_targets:vec![],
            cull:1,front_ccw:false,
        }],commands:vec![]};
        let legacy=encode_batch(&b).unwrap();
        assert_eq!(u32::from_le_bytes(legacy[28..32].try_into().unwrap()),28);
        let p=&decode_batch(&legacy).unwrap().pipelines[0];
        assert_eq!((p.cull,p.front_ccw),(1,false));
        for cull in 1..=3 {
            for front_ccw in [false,true] {
                b.pipelines[0].cull=cull;b.pipelines[0].front_ccw=front_ccw;
                let bytes=encode_batch(&b).unwrap();
                let decoded=decode_batch(&bytes).unwrap();
                assert_eq!((decoded.pipelines[0].cull,decoded.pipelines[0].front_ccw),(cull,front_ccw));
                assert_eq!(encode_batch(&decoded).unwrap(),bytes);
                let mut stream=Vec::new();write_stream_batch(&mut stream,&b).unwrap();
                let decoded=read_stream_batch(&mut stream.as_slice()).unwrap();
                assert_eq!((decoded.pipelines[0].cull,decoded.pipelines[0].front_ccw),(cull,front_ccw));
            }
        }
        b.pipelines[0].extra_targets.push(ColorTarget {format:87,blend:Blend::new(false,15,[0;6]).unwrap()});
        let bytes=encode_batch(&b).unwrap();
        let decoded=decode_batch(&bytes).unwrap();
        assert_eq!(decoded.pipelines[0].extra_targets[0].format,87);
        assert_eq!((decoded.pipelines[0].cull,decoded.pipelines[0].front_ccw),(3,true));
        for end in 0..bytes.len() {assert!(decode_batch(&bytes[..end]).is_err());}
        let n=bytes.len();
        for (offset,value) in [(n-12,0u32),(n-12,4),(n-8,2)] {
            let mut invalid=bytes.clone();
            invalid[offset..offset+4].copy_from_slice(&value.to_le_bytes());
            assert!(decode_batch(&invalid).is_err());
        }
        let mut missing_flag=bytes.clone();
        missing_flag[28..32].copy_from_slice(&(28u32 | 0x8000_0000).to_le_bytes());
        assert!(decode_batch(&missing_flag).is_err());
        for cull in [0,4,u32::MAX] {
            b.pipelines[0].cull=cull;
            assert!(encode_batch(&b).is_err());
            assert!(write_stream_batch(&mut Vec::new(),&b).is_err());
        }
        b.pipelines[0].cull=1;b.pipelines[0].format=0x4000_001c;
        assert!(encode_batch(&b).is_err());
    }

    #[test]
    fn multiple_render_targets_round_trip() {
        let mut b=batch();
        b.resources.push(Resource {id:3,state:4,kind:ResourceKind::Texture {width:1,height:1,format:87,mips:1},bytes:vec![0;4]});
        b.pipelines.push(Pipeline {id:7,vertex_shader:vec![1],pixel_shader:vec![2],vertex_stride:16,format:28,
            depth:None,blend:Blend::new(false,15,[0;6]).unwrap(),inputs:vec![],
            extra_targets:vec![ColorTarget {format:87,blend:Blend::new(true,5,[2,1,1,2,1,1]).unwrap()}],cull:1,front_ccw:false});
        b.commands.push(Command::Draw(Draw {target:2,extra_targets:vec![3],pipeline:7,vertex_buffer:1,vertex_offset:0,vertex_size:4,vertex_stride:16,
            vertex_count:3,instance_count:1,first_vertex:0,first_instance:0,viewport:[0.,0.,1.,1.,0.,1.],scissor:[0,0,1,1],
            topology:4,constants:vec![],texture:None,first_mip:0,mip_count:0,index:None,depth:None,linear_filter:false,
            constant_slot:None,texture_slot:None,sampler_slot:None,vertices:vec![],textures:vec![],samplers:vec![]}));
        let bytes=encode_batch(&b).unwrap();
        for end in 0..bytes.len() {assert!(decode_batch(&bytes[..end]).is_err());}
        assert_eq!(encode_batch(&decode_batch(&bytes).unwrap()).unwrap(),bytes);
        let mut streamed=vec![];
        write_stream_batch(&mut streamed,&b).unwrap();
        let decoded=read_stream_batch(&mut streamed.as_slice()).unwrap();
        assert_eq!(encode_batch(&decoded).unwrap(),bytes);
        let Some(Command::Draw(d))=b.commands.last_mut() else {unreachable!()};
        d.extra_targets[0]=2;
        assert!(encode_batch(&b).is_err());
    }

    #[test]
    fn multiple_sampled_bindings_round_trip_and_reject_invalid() {
        let mut b=batch();
        b.resources.push(Resource {id:3,state:0,kind:ResourceKind::Texture {width:1,height:1,format:28,mips:1},bytes:vec![1,2,3,4]});
        b.commands.push(Command::Draw(Draw {target:2,extra_targets:vec![],pipeline:7,vertex_buffer:1,vertex_offset:0,vertex_size:4,vertex_stride:16,
            vertex_count:3,instance_count:1,first_vertex:0,first_instance:0,viewport:[0.,0.,1.,1.,0.,1.],scissor:[0,0,1,1],
            topology:4,constants:vec![],texture:None,first_mip:0,mip_count:0,index:None,depth:None,linear_filter:false,
            constant_slot:None,texture_slot:None,sampler_slot:None,vertices:vec![],
            textures:vec![SampledTextureBinding {slot:1,offset:0,resource:2,first_mip:0,mip_count:1,array:None,cube:false,volume:false},SampledTextureBinding {slot:2,offset:0,resource:3,first_mip:0,mip_count:1,array:None,cube:false,volume:false}],
            samplers:vec![SamplerBinding {slot:3,offset:0,linear_filter:false,address:[3;3],mip_linear:false,min_lod:0.,max_lod:0.},SamplerBinding {slot:4,offset:0,linear_filter:true,address:[3;3],mip_linear:true,min_lod:0.,max_lod:0.}]}));
        let bytes=encode_batch(&b).unwrap();
        for end in 0..bytes.len() {assert!(decode_batch(&bytes[..end]).is_err());}
        assert_eq!(encode_batch(&decode_batch(&bytes).unwrap()).unwrap(),bytes);
        let Some(Command::Draw(d))=b.commands.last_mut() else {unreachable!()};
        d.samplers[1].slot=2;
        assert!(encode_batch(&b).is_err());
    }

    #[test]
    fn table_offsets_round_trip_and_reject_malformed_wire() {
        let mut b=batch();
        b.commands.push(Command::Draw(Draw {target:2,extra_targets:vec![],pipeline:7,vertex_buffer:1,vertex_offset:0,vertex_size:4,vertex_stride:16,
            vertex_count:3,instance_count:1,first_vertex:0,first_instance:0,viewport:[0.,0.,1.,1.,0.,1.],scissor:[0,0,1,1],
            topology:4,constants:vec![],texture:None,first_mip:0,mip_count:0,index:None,depth:None,linear_filter:false,
            constant_slot:None,texture_slot:None,sampler_slot:None,vertices:vec![],
            textures:vec![SampledTextureBinding {slot:1,offset:0,resource:2,first_mip:0,mip_count:1,array:None,cube:false,volume:false},
                SampledTextureBinding {slot:1,offset:2,resource:2,first_mip:0,mip_count:1,array:None,cube:false,volume:false}],
            samplers:vec![SamplerBinding {slot:3,offset:0,linear_filter:false,address:[3;3],mip_linear:false,min_lod:0.,max_lod:0.},SamplerBinding {slot:3,offset:1,linear_filter:true,address:[3;3],mip_linear:true,min_lod:0.,max_lod:0.}]}));
        let bytes=encode_batch(&b).unwrap();
        for end in 0..bytes.len() {assert!(decode_batch(&bytes[..end]).is_err());}
        assert_eq!(encode_batch(&decode_batch(&bytes).unwrap()).unwrap(),bytes);
        let mut stream=Vec::new();
        write_stream_batch(&mut stream,&b).unwrap();
        let decoded=read_stream_batch(&mut &stream[..]).unwrap();
        assert_eq!(encode_batch(&decoded).unwrap(),bytes);
        let n=bytes.len();
        for (offset,value) in [(n-48,0u32),(n-48,256),(n-24,1),(n-28,257),(n-8,0)] {
            let mut invalid=bytes.clone();
            invalid[offset..offset+4].copy_from_slice(&value.to_le_bytes());
            assert!(decode_batch(&invalid).is_err());
        }
        let Some(Command::Draw(d))=b.commands.last_mut() else {unreachable!()};
        d.texture=Some(2);
        assert!(encode_batch(&b).is_err());
    }

    #[test]
    fn addressed_sampler_wire_round_trip_and_validation() {
        let mut b=batch();
        b.commands=vec![Command::Draw(Draw {target:2,extra_targets:vec![],pipeline:7,vertex_buffer:1,vertex_offset:0,vertex_size:4,vertex_stride:16,
            vertex_count:3,instance_count:1,first_vertex:0,first_instance:0,viewport:[0.,0.,1.,1.,0.,1.],scissor:[0,0,1,1],
            topology:4,constants:vec![],texture:None,first_mip:0,mip_count:0,index:None,depth:None,linear_filter:false,
            constant_slot:None,texture_slot:None,sampler_slot:None,vertices:vec![],
            textures:vec![SampledTextureBinding {slot:1,offset:2,resource:2,first_mip:0,mip_count:1,array:None,cube:false,volume:false}],
            samplers:vec![SamplerBinding {slot:3,offset:4,linear_filter:true,address:[3;3],mip_linear:true,min_lod:0.,max_lod:0.}]})];
        let opcode=encode_results(&b.resources).unwrap().len()+8;
        let legacy=encode_batch(&b).unwrap();
        assert_eq!(&legacy[opcode..opcode+4],&11u32.to_le_bytes());
        let decoded=decode_batch(&legacy).unwrap();
        let Command::Draw(d)=&decoded.commands[0] else {unreachable!()};
        assert_eq!(d.samplers[0].address,[3;3]);
        assert_eq!(encode_batch(&decoded).unwrap(),legacy);
        for extra in [false,true] {
            if extra {
                b.resources.push(Resource {id:3,state:0,kind:ResourceKind::Texture {width:1,height:1,format:28,mips:1},bytes:vec![0;4]});
            }
            let Command::Draw(d)=&mut b.commands[0] else {unreachable!()};
            d.samplers[0].address=[1,2,3];
            if extra {d.extra_targets.push(3);}
            let bytes=encode_batch(&b).unwrap();
            let opcode=encode_results(&b.resources).unwrap().len()+8;
            assert_eq!(&bytes[opcode..opcode+4],&13u32.to_le_bytes());
            let decoded=decode_batch(&bytes).unwrap();
            let Command::Draw(d)=&decoded.commands[0] else {unreachable!()};
            assert_eq!(d.samplers[0].address,[1,2,3]);
            assert!(d.samplers[0].linear_filter);
            assert_eq!(d.samplers[0].offset,4);
            assert_eq!(d.extra_targets.len(),extra as usize);
            assert_eq!(encode_batch(&decoded).unwrap(),bytes);
            let mut stream=Vec::new();write_stream_batch(&mut stream,&b).unwrap();
            assert_eq!(encode_batch(&read_stream_batch(&mut stream.as_slice()).unwrap()).unwrap(),bytes);
            for end in 0..bytes.len() {assert!(decode_batch(&bytes[..end]).is_err());}
            let packed=bytes.len()-if extra {16} else {8};
            for value in [0u32,0x80,0x79,0x67,0x1f,u32::MAX] {
                let mut invalid=bytes.clone();
                invalid[packed..packed+4].copy_from_slice(&value.to_le_bytes());
                assert!(decode_batch(&invalid).is_err(),"packed {value:#x}");
            }
        }
        for invalid in [0,4,u32::MAX] {
            let Command::Draw(d)=&mut b.commands[0] else {unreachable!()};
            d.samplers[0].address=[1,invalid,3];
            assert!(encode_batch(&b).is_err());
            assert!(write_stream_batch(&mut Vec::new(),&b).is_err());
        }
    }

    #[test]
    fn sampler_lod_wire_round_trip_and_rejects_malformed() {
        let mut b=batch();
        b.commands=vec![Command::Draw(Draw {target:2,extra_targets:vec![],pipeline:7,vertex_buffer:1,vertex_offset:0,vertex_size:4,vertex_stride:16,
            vertex_count:3,instance_count:1,first_vertex:0,first_instance:0,viewport:[0.,0.,1.,1.,0.,1.],scissor:[0,0,1,1],
            topology:4,constants:vec![],texture:None,first_mip:0,mip_count:0,index:None,depth:None,linear_filter:false,
            constant_slot:None,texture_slot:None,sampler_slot:None,vertices:vec![],textures:vec![],
            samplers:vec![SamplerBinding {slot:3,offset:4,linear_filter:true,address:[1,2,3],mip_linear:true,min_lod:0.,max_lod:0.}]})];
        let opcode=encode_results(&b.resources).unwrap().len()+8;
        let legacy=encode_batch(&b).unwrap();
        assert_eq!(&legacy[opcode..opcode+4],&13u32.to_le_bytes());
        let decoded=decode_batch(&legacy).unwrap();
        let Command::Draw(d)=&decoded.commands[0] else {unreachable!()};
        assert_eq!((d.samplers[0].mip_linear,d.samplers[0].min_lod,d.samplers[0].max_lod),(true,0.,0.));
        let Command::Draw(d)=&mut b.commands[0] else {unreachable!()};
        d.samplers[0].mip_linear=false;d.samplers[0].min_lod=0.5;d.samplers[0].max_lod=4.0;
        let bytes=encode_batch(&b).unwrap();
        assert_eq!(&bytes[opcode..opcode+4],&14u32.to_le_bytes());
        let decoded=decode_batch(&bytes).unwrap();
        let Command::Draw(d)=&decoded.commands[0] else {unreachable!()};
        let s=&d.samplers[0];
        assert_eq!((s.address,s.offset,s.linear_filter,s.mip_linear,s.min_lod,s.max_lod),([1,2,3],4,true,false,0.5,4.0));
        assert_eq!(encode_batch(&decoded).unwrap(),bytes);
        let mut stream=Vec::new();write_stream_batch(&mut stream,&b).unwrap();
        assert_eq!(encode_batch(&read_stream_batch(&mut stream.as_slice()).unwrap()).unwrap(),bytes);
        for end in 0..bytes.len() {assert!(decode_batch(&bytes[..end]).is_err());}
        let n=bytes.len();
        for (offset,value) in [(n-20,0x80u32),(n-16,2),(n-12,f32::NAN.to_bits()),
            (n-12,(-1.0f32).to_bits()),(n-8,f32::INFINITY.to_bits()),(n-8,0.25f32.to_bits())] {
            let mut invalid=bytes.clone();invalid[offset..offset+4].copy_from_slice(&value.to_le_bytes());
            assert!(decode_batch(&invalid).is_err());
        }
        for (min,max) in [(f32::NAN,1.),(-1.,1.),(2.,1.),(0.,f32::INFINITY)] {
            let Command::Draw(d)=&mut b.commands[0] else {unreachable!()};
            d.samplers[0].min_lod=min;d.samplers[0].max_lod=max;
            assert!(encode_batch(&b).is_err());
            assert!(write_stream_batch(&mut Vec::new(),&b).is_err());
        }
    }

    #[test]
    fn vertex_input_wire_round_trip() {
        let mut b=batch();
        b.pipelines.push(Pipeline {id:7,vertex_shader:vec![1],pixel_shader:vec![2],vertex_stride:24,format:28,
            depth:None,blend:Blend::new(false,15,[0;6]).unwrap(),inputs:vec![
                VertexInput {semantic:"TEXCOORD".into(),semantic_index:0,format:34,slot:1,offset:0,per_instance:true,step_rate:2},
                VertexInput {semantic:"POSITION".into(),semantic_index:0,format:2,slot:0,offset:4,per_instance:false,step_rate:0},
            ],extra_targets:vec![],cull:1,front_ccw:false});
        b.commands.push(Command::Draw(Draw {target:2,extra_targets:vec![],pipeline:7,vertex_buffer:1,vertex_offset:0,vertex_size:4,vertex_stride:16,
            vertex_count:3,instance_count:3,first_vertex:0,first_instance:2,viewport:[0.,0.,1.,1.,0.,1.],scissor:[0,0,1,1],
            topology:4,constants:vec![],texture:None,first_mip:0,mip_count:0,index:None,depth:None,linear_filter:false,
            constant_slot:None,texture_slot:None,sampler_slot:None,
            vertices:vec![VertexView {slot:0,buffer:1,offset:0,size:4,stride:16},VertexView {slot:1,buffer:1,offset:0,size:4,stride:4}],textures:vec![],samplers:vec![]}));
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
            blend:Blend::new(false,15,[0;6]).unwrap(),inputs:vec![],extra_targets:vec![],cull:1,front_ccw:false});
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
        b.commands=vec![Command::Draw(Draw {target:2,extra_targets:vec![],pipeline:7,vertex_buffer:1,vertex_offset:0,vertex_size:4,vertex_stride:16,
            vertex_count:3,instance_count:1,first_vertex:0,first_instance:0,viewport:[0.,0.,1.,1.,0.,1.],scissor:[0,0,1,1],
            topology:4,constants:vec![],texture:Some(2),first_mip:1,mip_count:2,index:None,depth:None,linear_filter:false,
            constant_slot:None,texture_slot:Some(0),sampler_slot:Some(0),vertices:vec![],textures:vec![],samplers:vec![]})];
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
        b.commands=vec![Command::CopyTexture(CopyTexture {src:1,dst:2,offset:0,pitch:12,width:3,height:1,depth:1,format:28,mip:1,slice:0,buffer_to_texture:true})];
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
