//! Restricted Wine D3D12.X bridge. Graphics supports only the fixed 224-byte
//! POSITION float4/RGBA8 pipeline stream, one full clear, one nonindexed
//! triangle, full viewport/scissor, contiguous slot-3 constants, one
//! render-target-to-copy-source barrier and one full placed-footprint copy.
//! Commands are synchronous at submission; all wire data is copied.
use std::{
    cell::UnsafeCell,
    collections::HashMap,
    ffi::c_void,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};
use crate::object::{guid, HRESULT, Iid, Object, Table, This, E_INVALIDARG, E_NOINTERFACE, S_OK};

const FAIL: HRESULT = 0x80004005u32 as i32;
const MAGIC: u32 = 0x31474d56;
const MAX_SHADER: usize = 4 * 1024 * 1024;
const MAX_BUFFER: usize = 16 * 1024 * 1024;
const UNKNOWN: Iid = guid(0, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
const HEAP: Iid = guid(0x8efb471d, 0x616c, 0x4f49, [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51]);
const RESOURCE: Iid = guid(0x696442be, 0xa72e, 0x4059, [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);
const PIPELINE: Iid = guid(0x765a30f3, 0xf624, 0x4c6f, [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45]);
const ALLOCATOR: Iid = guid(0x6102dee4, 0xaf59, 0x4b09, [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24]);
const LIST: Iid = guid(0x7116d91c, 0xe7e4, 0x47ce, [0xb8, 0xc6, 0xec, 0x81, 0x68, 0xf4, 0x37, 0xe5]);
const GRAPHICS_LIST: Iid = guid(0x5b160d0f, 0xac1b, 0x4185, [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55]);
const QUEUE: Iid = guid(0x0ec870a6, 0x5d7e, 0x4c22, [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed]);

#[derive(Clone)]
struct Config {
    addr: SocketAddr,
    token: [u8; 16],
}
fn config() -> Result<Config, String> {
    let addr: SocketAddr = std::env::var("VICTORY_WINE_METAL_ADDR")
        .map_err(|e| e.to_string())?
        .parse::<SocketAddr>()
        .map_err(|e| e.to_string())?;
    if !addr.ip().is_loopback() || addr.port() == 0 {
        return Err("proxy address must be a loopback socket".into());
    }
    let s = std::env::var("VICTORY_WINE_METAL_TOKEN").map_err(|e| e.to_string())?;
    if s.len() != 32 || !s.is_ascii() {
        return Err("token must be 32 hex digits".into());
    }
    let mut token = [0; 16];
    for (i, b) in token.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).map_err(|_| "invalid token")?;
    }
    Ok(Config { addr, token })
}

// These objects keep their COM identity in a registry; recorded jobs retain
// their resources and bytecode independently of guest Release calls.
#[repr(C)]
struct Header {
    object: Object,
    inline: [u64; 180],
}
struct Entry {
    refs: u32,
    data: Data,
}
#[derive(Clone)]
enum Data {
    Device(Arc<Config>),
    Heap(Arc<Mutex<Option<Arc<Buffer>>>>),
    Buffer(Arc<Buffer>),
    Pipeline(Arc<Vec<u8>>),
    Allocator(u32),
    List(Arc<Mutex<ListState>>),
    Queue(Arc<Config>, u32),
    Texture(Arc<Texture>),
    Rtv(Arc<Mutex<(Option<Arc<Texture>>, u64)>>),
    GraphicsPipeline(Arc<GraphicsPipeline>),
}
struct Buffer {
    bytes: Box<[UnsafeCell<u8>]>,
}
impl Buffer {
    fn len(&self) -> usize {
        self.bytes.len()
    }
    fn ptr(&self) -> *mut u8 {
        self.bytes.as_ptr().cast::<u8>() as *mut u8
    }
}
// Guest access to mapped memory and queue submission must be synchronized by
// the guest, as with an ordinary persistently mapped D3D resource.
unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}
struct Texture { width: u32, height: u32 }
struct GraphicsPipeline { vs: Vec<u8>, ps: Vec<u8> }
#[derive(Clone)]
struct GraphicsJob {
    pipeline: Arc<GraphicsPipeline>, texture: Arc<Texture>, vertices: Arc<Buffer>,
    vertex_offset: usize, constants: Vec<u32>, clear: [f32; 4],
    rtv: Arc<Mutex<(Option<Arc<Texture>>, u64)>>, revision: u64,
    output: Arc<Buffer>, offset: usize, pitch: usize,
}
#[derive(Default)]
struct GraphicsState {
    pipeline: Option<Arc<GraphicsPipeline>>,
    clear: Option<(Arc<Mutex<(Option<Arc<Texture>>, u64)>>, Arc<Texture>, u64, [f32; 4])>,
    target: Option<(Arc<Mutex<(Option<Arc<Texture>>, u64)>>, Arc<Texture>, u64)>,
    viewport: Option<[f32; 6]>, scissor: Option<[i32; 4]>, topology: bool,
    vertices: Option<(Arc<Buffer>, usize)>, constants: Vec<Option<u32>>,
    drawn: bool, barrier: bool, job: Option<GraphicsJob>,
}
#[derive(Clone)]
struct Job {
    shader: Arc<Vec<u8>>,
    heap: Arc<Mutex<Option<Arc<Buffer>>>>,
    buffer: Arc<Buffer>,
    constants: Vec<u32>,
    groups: [u32; 3],
}
#[derive(Default)]
struct ListState {
    closed: bool,
    invalid: bool,
    pipeline: Option<Arc<Vec<u8>>>,
    heap: Option<(usize, Arc<Mutex<Option<Arc<Buffer>>>>)>,
    table: bool,
    constants: Vec<Option<u32>>,
    jobs: Vec<Job>,
    kind: u32,
    graphics: GraphicsState,
}
static OBJECTS: LazyLock<Mutex<HashMap<usize, Entry>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
fn objects() -> &'static Mutex<HashMap<usize, Entry>> {
    &OBJECTS
}
static SUBMISSION: Mutex<()> = Mutex::new(());
fn data(p: This) -> Option<Data> {
    objects().lock().unwrap().get(&(p as usize)).map(|e| e.data.clone())
}
fn trap(message: &str) -> ! {
    eprintln!("Wine Metal proxy: {message}");
    std::process::abort()
}
fn new(data: Data, table: &'static Table<160>, iid: *const Iid, expected: Iid, out: *mut This) -> HRESULT {
    if iid.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    unsafe {
        *out = std::ptr::null();
        if *iid != expected && *iid != UNKNOWN && !(expected == LIST && *iid == GRAPHICS_LIST) {
            return E_NOINTERFACE;
        }
    }
    let mut header = Box::new(Header { object: Object::new(table), inline: [0; 180] });
    if matches!(data, Data::Device(_)) {
        header.inline[1] = 1;
    }
    let ptr = Box::into_raw(header) as This;
    objects().lock().unwrap().insert(ptr as usize, Entry { refs: 1, data });
    unsafe { *out = ptr; }
    S_OK
}
extern "system" fn query(this: This, iid: *const Iid, out: *mut This) -> HRESULT {
    if iid.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    unsafe { *out = std::ptr::null(); }
    let mut map = objects().lock().unwrap();
    let Some(entry) = map.get_mut(&(this as usize)) else {
        return E_INVALIDARG;
    };
    let expected = match &entry.data {
        Data::Device(_) => crate::device::IID,
        Data::Heap(_) => HEAP,
        Data::Buffer(_) => RESOURCE,
        Data::Pipeline(_) => PIPELINE,
        Data::Allocator(_) => ALLOCATOR,
        Data::List(_) => LIST,
        Data::Queue(_, _) => QUEUE,
        Data::Texture(_) => RESOURCE,
        Data::Rtv(_) => HEAP,
        Data::GraphicsPipeline(_) => PIPELINE,
    };
    if unsafe { *iid != expected && *iid != UNKNOWN && !(expected == LIST && *iid == GRAPHICS_LIST) } {
        return E_NOINTERFACE;
    }
    entry.refs = entry.refs.checked_add(1).unwrap_or_else(|| trap("reference overflow"));
    unsafe { *out = this; }
    S_OK
}
extern "system" fn add_ref(this: This) -> u32 {
    let mut m = objects().lock().unwrap();
    let e = m.get_mut(&(this as usize)).unwrap_or_else(|| trap("invalid AddRef"));
    e.refs = e.refs.checked_add(1).unwrap_or_else(|| trap("reference overflow"));
    e.refs
}
extern "system" fn release(this: This) -> u32 {
    let mut m = objects().lock().unwrap();
    let e = m.get_mut(&(this as usize)).unwrap_or_else(|| trap("invalid Release"));
    e.refs -= 1;
    let n = e.refs;
    if n == 0 {
        m.remove(&(this as usize));
        unsafe { drop(Box::from_raw(this as *mut Header)); }
    }
    n
}
extern "system" fn unsupported() -> ! {
    trap("unsupported COM method")
}
const fn base() -> [*const (); 160] {
    let mut t = [unsupported as *const (); 160];
    t[0] = query as *const ();
    t[1] = add_ref as *const ();
    t[2] = release as *const ();
    t
}
static DEVICE_TABLE: Table<160> = {
    let mut t = base();
    t[8] = create_queue as *const ();
    t[9] = create_allocator as *const ();
    t[11] = create_pipeline as *const ();
    t[12] = create_list as *const ();
    t[14] = create_heap as *const ();
    t[20] = create_rtv as *const ();
    t[86] = create_graphics_pipeline as *const ();
    t[19] = create_uav as *const ();
    t[27] = create_resource as *const ();
    Table(t)
};
static HEAP_TABLE: Table<160> = {
    let mut t = base();
    t[9] = heap_handle as *const ();
    t[10] = heap_handle as *const ();
    Table(t)
};
static BUFFER_TABLE: Table<160> = {
    let mut t = base();
    t[8] = map as *const ();
    t[9] = unmap as *const ();
    t[11] = gpu_address as *const ();
    Table(t)
};
static PIPELINE_TABLE: Table<160> = Table(base());
static ALLOCATOR_TABLE: Table<160> = Table(base());
static LIST_TABLE: Table<160> = {
    let mut t = base();
    t[8] = list_type as *const ();
    t[9] = close as *const ();
    t[12] = draw as *const ();
    t[16] = copy_texture as *const ();
    t[20] = topology as *const ();
    t[21] = viewport as *const ();
    t[22] = scissor as *const ();
    t[26] = barrier as *const ();
    t[38] = graphics_constant as *const ();
    t[40] = graphics_constants as *const ();
    t[48] = vertices as *const ();
    t[50] = targets as *const ();
    t[52] = clear_rtv as *const ();
    t[14] = dispatch as *const ();
    t[25] = set_pipeline as *const ();
    t[32] = set_heaps as *const ();
    t[35] = set_table as *const ();
    t[39] = set_constants as *const ();
    Table(t)
};
static QUEUE_TABLE: Table<160> = {
    let mut t = base();
    t[10] = execute as *const ();
    Table(t)
};

pub(super) fn create(iid: &Iid, out: *mut This) -> HRESULT {
    unsafe { *out = std::ptr::null(); }
    match config() {
        Ok(c) if *iid == crate::device::IID => {
            new(Data::Device(Arc::new(c)), &DEVICE_TABLE, iid, crate::device::IID, out)
        }
        Ok(_) => E_NOINTERFACE,
        Err(e) => {
            eprintln!("Wine Metal proxy: {e}");
            E_INVALIDARG
        }
    }
}
fn device(p: This) -> Option<Arc<Config>> {
    if let Some(Data::Device(c)) = data(p) {
        Some(c)
    } else {
        None
    }
}
#[repr(C)]
struct HeapDesc {
    kind: u32,
    count: u32,
    flags: u32,
    nodes: u32,
}
extern "system" fn create_heap(this: This, d: *const HeapDesc, iid: *const Iid, out: *mut This) -> HRESULT {
    let Some(d) = (unsafe { d.as_ref() }) else {
        return E_INVALIDARG;
    };
    if device(this).is_none() || d.count != 1 || d.nodes != 0 {
        return E_INVALIDARG;
    }
    if d.kind == 2 && d.flags == 0 {
        return new(Data::Rtv(Arc::new(Mutex::new((None, 0)))), &HEAP_TABLE, iid, HEAP, out);
    }
    if d.kind != 0 || d.flags != 1 { return E_INVALIDARG; }
    new(Data::Heap(Arc::new(Mutex::new(None))), &HEAP_TABLE, iid, HEAP, out)
}
extern "system" fn heap_handle(this: This) -> u64 {
    if !matches!(data(this), Some(Data::Heap(_)) | Some(Data::Rtv(_))) {
        trap("invalid heap");
    }
    this as u64
}
#[repr(C)]
struct HeapProps {
    kind: u32,
    cpu: u32,
    pool: u32,
    creation: u32,
    visible: u32,
}
#[repr(C)]
struct ResourceDesc {
    dimension: u32,
    alignment: u64,
    width: u64,
    height: u32,
    depth: u16,
    mips: u16,
    format: u32,
    samples: u32,
    quality: u32,
    layout: u32,
    flags: u32,
}
extern "system" fn create_resource(
    this: This,
    hp: *const HeapProps,
    flags: u32,
    rd: *const ResourceDesc,
    state: u32,
    clear: *const c_void,
    iid: *const Iid,
    out: *mut This,
) -> HRESULT {
    let (Some(h), Some(r)) = (unsafe { hp.as_ref() }, unsafe { rd.as_ref() }) else {
        return E_INVALIDARG;
    };
    if device(this).is_none() || h.kind != 1 || h.cpu != 0 || h.pool != 0
        || h.creation > 1 || h.visible > 1 || flags != 0 || !clear.is_null()
        || r.alignment != 0 || r.width == 0 || r.depth != 1 || r.mips != 1
        || r.samples != 1 || r.quality != 0 {
        return E_INVALIDARG;
    }
    if r.dimension == 3 {
        let Some(size) = r.width.checked_mul(r.height as u64).and_then(|n| n.checked_mul(4)) else { return E_INVALIDARG; };
        if r.width > u32::MAX as u64 || r.height == 0 || size > MAX_BUFFER as u64
            || r.format != 28 || r.layout != 1 || r.flags != 1 || state != 4 {
            return E_INVALIDARG;
        }
        return new(Data::Texture(Arc::new(Texture { width: r.width as u32, height: r.height })), &BUFFER_TABLE, iid, RESOURCE, out);
    }
    if r.dimension != 1 || r.width > MAX_BUFFER as u64 || r.height != 1 || r.format != 0
        || r.layout != 1 || !matches!(r.flags, 0 | 4) || state != 0 {
        return E_INVALIDARG;
    }
    let bytes = (0..r.width).map(|_| UnsafeCell::new(0)).collect::<Vec<_>>().into_boxed_slice();
    new(Data::Buffer(Arc::new(Buffer { bytes })), &BUFFER_TABLE, iid, RESOURCE, out)
}
#[repr(C)]
struct Range {
    begin: usize,
    end: usize,
}
extern "system" fn map(this: This, sub: u32, range: *const Range, out: *mut *mut c_void) -> HRESULT {
    if out.is_null() || sub != 0 {
        return E_INVALIDARG;
    }
    unsafe { *out = std::ptr::null_mut(); }
    if let Some(Data::Buffer(b)) = data(this) {
        if let Some(r) = unsafe { range.as_ref() } {
            if r.begin > r.end || r.end > b.len() {
                return E_INVALIDARG;
            }
        }
        unsafe { *out = b.ptr().cast(); }
        S_OK
    } else {
        E_INVALIDARG
    }
}
extern "system" fn unmap(this: This, sub: u32, _range: *const c_void) {
    if sub != 0 || !matches!(data(this), Some(Data::Buffer(_))) {
        trap("invalid Unmap");
    }
}
#[repr(C)]
struct UavDesc {
    format: u32,
    dimension: u32,
    first: u64,
    elements: u32,
    stride: u32,
    counter: u64,
    flags: u32,
}
extern "system" fn create_uav(
    this: This,
    res: This,
    counter: This,
    desc: *const UavDesc,
    handle: u64,
) {
    let (Some(Data::Buffer(b)), Some(Data::Heap(h)), Some(d)) =
        (data(res), data(handle as This), unsafe { desc.as_ref() })
    else {
        trap("invalid UAV arguments");
    };
    if device(this).is_none() || !counter.is_null() || d.format != 39 || d.dimension != 1
        || d.first != 0 || d.elements as usize * 4 != b.len() || d.stride != 0
        || d.counter != 0 || d.flags != 1
    {
        trap("unsupported UAV view");
    }
    *h.lock().unwrap() = Some(b);
}
#[repr(C)]
struct ComputeDesc {
    root: This,
    code: *const u8,
    size: usize,
}
extern "system" fn create_pipeline(this: This, d: *const ComputeDesc, iid: *const Iid, out: *mut This) -> HRESULT {
    let Some(d) = (unsafe { d.as_ref() }) else {
        return E_INVALIDARG;
    };
    if device(this).is_none() || !d.root.is_null() || d.code.is_null() || d.size == 0 || d.size > MAX_SHADER {
        return E_INVALIDARG;
    }
    let bytes = unsafe { std::slice::from_raw_parts(d.code, d.size) }.to_vec();
    new(Data::Pipeline(Arc::new(bytes)), &PIPELINE_TABLE, iid, PIPELINE, out)
}
extern "system" fn create_allocator(this: This, kind: u32, iid: *const Iid, out: *mut This) -> HRESULT {
    if device(this).is_none() || (kind != 0 && kind != 2) {
        return E_INVALIDARG;
    }
    new(Data::Allocator(kind), &ALLOCATOR_TABLE, iid, ALLOCATOR, out)
}
extern "system" fn create_list(
    this: This,
    nodes: u32,
    kind: u32,
    alloc: This,
    pso: This,
    iid: *const Iid,
    out: *mut This,
) -> HRESULT {
    if device(this).is_none() || nodes != 0 || (kind != 0 && kind != 2) || !matches!(data(alloc), Some(Data::Allocator(k)) if k == kind) {
        return E_INVALIDARG;
    }
    let mut state = ListState { kind, ..Default::default() };
    if !pso.is_null() {
        match data(pso) {
            Some(Data::Pipeline(p)) if kind == 2 => state.pipeline = Some(p),
            Some(Data::GraphicsPipeline(p)) if kind == 0 => state.graphics.pipeline = Some(p),
            _ => return E_INVALIDARG,
        }
    }
    new(Data::List(Arc::new(Mutex::new(state))), &LIST_TABLE, iid, LIST, out,
    )
}
#[repr(C)]
struct QueueDesc {
    kind: u32,
    priority: i32,
    flags: u32,
    nodes: u32,
}
extern "system" fn create_queue(this: This, d: *const QueueDesc, iid: *const Iid, out: *mut This) -> HRESULT {
    let (Some(c), Some(d)) = (device(this), unsafe { d.as_ref() }) else {
        return E_INVALIDARG;
    };
    if (d.kind != 0 && d.kind != 2) || d.priority != 0 || d.flags != 0 || d.nodes != 0 {
        return E_INVALIDARG;
    }
    new(Data::Queue(c, d.kind), &QUEUE_TABLE, iid, QUEUE, out)
}
fn list(this: This) -> Arc<Mutex<ListState>> {
    if let Some(Data::List(l)) = data(this) {
        l
    } else {
        trap("invalid list")
    }
}
extern "system" fn list_type(this: This) -> u32 {
    list(this).lock().unwrap().kind
}
extern "system" fn close(this: This) -> HRESULT {
    let l = list(this);
    let mut l = l.lock().unwrap();
    if l.closed || l.invalid || (l.kind == 0 && l.graphics.job.is_none()) {
        return FAIL;
    }
    l.closed = true;
    S_OK
}
extern "system" fn set_pipeline(this: This, p: This) {
    let l = list(this);
    let mut l = l.lock().unwrap();
    if l.closed {
        trap("closed list");
    }
    match data(p) {
        Some(Data::Pipeline(p)) if l.kind == 2 => l.pipeline = Some(p),
        Some(Data::GraphicsPipeline(p)) if l.kind == 0 && !l.graphics.drawn => l.graphics.pipeline = Some(p),
        _ => l.invalid = true,
    }
}
extern "system" fn set_heaps(this: This, n: u32, heaps: *const This) {
    let l = list(this);
    let mut l = l.lock().unwrap();
    if l.closed {
        trap("closed list");
    }
    if l.kind != 2 || n != 1 || heaps.is_null() {
        l.invalid = true;
        return;
    }
    let p = unsafe { *heaps };
    if let Some(Data::Heap(h)) = data(p) {
        l.heap = Some((p as usize, h));
        l.table = false;
    } else {
        l.invalid = true;
    }
}
extern "system" fn set_table(this: This, slot: u32, handle: u64) {
    let l = list(this);
    let mut l = l.lock().unwrap();
    if l.closed {
        trap("closed list");
    }
    if l.kind != 2 || slot != 1 || l.heap.as_ref().map(|h| h.0) != Some(handle as usize) {
        l.invalid = true;
    } else {
        l.table = true;
    }
}
extern "system" fn set_constants(this: This, slot: u32, n: u32, values: *const u32, offset: u32) {
    let l = list(this);
    let mut l = l.lock().unwrap();
    if l.closed {
        trap("closed list");
    }
    let end = (offset as usize).checked_add(n as usize);
    if l.kind != 2 || slot != 0 || values.is_null() || end.is_none() || end.unwrap() > 64 {
        l.invalid = true;
        return;
    }
    let end = end.unwrap();
    if l.constants.len() < end {
        l.constants.resize(end, None);
    }
    for (dst, value) in l.constants[offset as usize..end]
        .iter_mut()
        .zip(unsafe { std::slice::from_raw_parts(values, n as usize) })
    {
        *dst = Some(*value);
    }
}
// Retain both objects and reject descriptor changes before submission.
extern "system" fn dispatch(this: This, x: u32, y: u32, z: u32) {
    let l = list(this);
    let mut l = l.lock().unwrap();
    if l.closed {
        trap("closed list");
    }
    let heap = l.heap.as_ref().map(|h| h.1.clone());
    let buffer = heap.as_ref().and_then(|h| h.lock().unwrap().clone());
    let constants: Option<Vec<u32>> = l.constants.iter().copied().collect();
    if l.kind != 2 || x == 0 || y == 0 || z == 0 || !l.table || l.pipeline.is_none()
        || buffer.is_none() || constants.is_none()
    {
        l.invalid = true;
        return;
    }
    let job = Job {
        shader: l.pipeline.as_ref().unwrap().clone(),
        heap: heap.unwrap(),
        buffer: buffer.unwrap(),
        constants: constants.unwrap(),
        groups: [x, y, z],
    };
    l.jobs.push(job);
}
fn run(c: &Config, j: &Job) -> Result<(), String> {
    let mut s = TcpStream::connect_timeout(&c.addr, Duration::from_secs(10))
        .map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(60))).map_err(|e| e.to_string())?;
    s.set_write_timeout(Some(Duration::from_secs(60))).map_err(|e| e.to_string())?;
    for v in [
        MAGIC, 1, j.shader.len() as u32, j.buffer.len() as u32, j.constants.len() as u32,
        j.groups[0], j.groups[1], j.groups[2],
    ] {
        s.write_all(&v.to_le_bytes()).map_err(|e| e.to_string())?;
    }
    let input = unsafe { std::slice::from_raw_parts(j.buffer.ptr(), j.buffer.len()) };
    s.write_all(&c.token)
        .and_then(|_| s.write_all(&j.shader))
        .and_then(|_| s.write_all(input))
        .map_err(|e| e.to_string())?;
    for v in &j.constants {
        s.write_all(&v.to_le_bytes()).map_err(|e| e.to_string())?;
    }
    let mut header = [0u8; 16];
    s.read_exact(&mut header).map_err(|e| e.to_string())?;
    let word = |n: usize| u32::from_le_bytes(header[n..n + 4].try_into().unwrap());
    let (magic, status, len, msg) = (word(0), word(4), word(8) as usize, word(12) as usize);
    if magic != MAGIC || len > MAX_BUFFER || msg > 4096
        || (status == 0 && (len != j.buffer.len() || msg != 0))
        || (status != 0 && len != 0)
    {
        return Err("invalid proxy response".into());
    }
    let mut bytes = vec![0; len];
    let mut message = vec![0; msg];
    s.read_exact(&mut bytes).and_then(|_| s.read_exact(&mut message)).map_err(|e| e.to_string())?;
    let message = String::from_utf8(message).map_err(|e| e.to_string())?;
    if status != 0 {
        return Err(format!("proxy status {status}: {message}"));
    }
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), j.buffer.ptr(), len); }
    tracing::info!("Wine Metal proxy connected");
    Ok(())
}
extern "system" fn execute(this: This, count: u32, lists: *const This) {
    let Some(Data::Queue(c, kind)) = data(this) else {
        trap("invalid queue");
    };
    if count == 0 || lists.is_null() {
        trap("invalid command lists");
    }
    let _submission = SUBMISSION.lock().unwrap();
    for &p in unsafe { std::slice::from_raw_parts(lists, count as usize) } {
        let l = list(p);
        let l = l.lock().unwrap();
        if !l.closed || l.invalid || l.kind != kind {
            trap("invalid or open command list");
        }
        if l.kind == 0 {
            let j = l.graphics.job.as_ref().unwrap_or_else(|| trap("missing graphics copy"));
            let descriptor = j.rtv.lock().unwrap();
            if descriptor.1 != j.revision || !descriptor.0.as_ref().is_some_and(|t| Arc::ptr_eq(t, &j.texture)) { trap("RTV descriptor changed"); }
            if let Err(e) = run_graphics(&c, j) { trap(&e); }
            continue;
        }
        for j in &l.jobs {
            let descriptor = j.heap.lock().unwrap();
            if !descriptor.as_ref().is_some_and(|b| Arc::ptr_eq(b, &j.buffer)) {
                trap("UAV descriptor changed after dispatch");
            }
            if let Err(e) = run(&c, j) {
                trap(&e);
            }
        }
    }
}

#[repr(C)]
struct StreamDesc { size: usize, stream: *const u8 }
#[repr(C)]
struct InputElement { name: *const u8, index: u32, format: u32, slot: u32, offset: u32, class: u32, step: u32 }
fn stream_ptr(bytes: &[u8], at: usize) -> usize {
    usize::from_ne_bytes(bytes[at..at + 8].try_into().unwrap())
}
extern "system" fn create_graphics_pipeline(this: This, desc: *const StreamDesc, iid: *const Iid, out: *mut This) -> HRESULT {
    let Some(d) = (unsafe { desc.as_ref() }) else { return E_INVALIDARG; };
    if device(this).is_none() || d.size != 224 || d.stream.is_null() { return E_INVALIDARG; }
    let b = unsafe { std::slice::from_raw_parts(d.stream, d.size) };
    let mut expected = [0u8; 224];
    for (at, value) in [(0,1u32),(24,2),(48,10),(52,3),(56,1),(76,1),(96,11),(152,12),(168,1),(176,14),(180,3),(184,15),(188,28),(220,1)] {
        expected[at..at+4].copy_from_slice(&value.to_ne_bytes());
    }
    // Only the three pointer-bearing fields and shader lengths vary.
    for (start, end) in [(8,24),(32,48),(160,168)] { expected[start..end].copy_from_slice(&b[start..end]); }
    if b != expected { return E_INVALIDARG; }
    let (vp, vl, pp, pl, ep) = (stream_ptr(b,8),stream_ptr(b,16),stream_ptr(b,32),stream_ptr(b,40),stream_ptr(b,160));
    if vp == 0 || pp == 0 || ep == 0 || vl == 0 || pl == 0 || vl > MAX_SHADER || pl > MAX_SHADER { return E_INVALIDARG; }
    let e = unsafe { &*(ep as *const InputElement) };
    if e.name.is_null() || e.index != 0 || e.format != 2 || e.slot != 0 || e.offset != 0 || e.class != 0 || e.step != 0 { return E_INVALIDARG; }
    if unsafe { std::slice::from_raw_parts(e.name, 9) } != b"POSITION\0" { return E_INVALIDARG; }
    let p = GraphicsPipeline { vs: unsafe { std::slice::from_raw_parts(vp as *const u8,vl) }.to_vec(), ps: unsafe { std::slice::from_raw_parts(pp as *const u8,pl) }.to_vec() };
    new(Data::GraphicsPipeline(Arc::new(p)), &PIPELINE_TABLE, iid, PIPELINE, out)
}
extern "system" fn gpu_address(this: This) -> u64 {
    match data(this) { Some(Data::Buffer(b)) => b.ptr() as u64, _ => trap("invalid GPU address") }
}
extern "system" fn create_rtv(this: This, resource: This, desc: *const c_void, handle: u64) {
    let (Some(Data::Texture(t)), Some(Data::Rtv(h))) = (data(resource), data(handle as This)) else { trap("invalid RTV"); };
    if device(this).is_none() || !desc.is_null() { trap("unsupported RTV"); }
    let mut d = h.lock().unwrap();
    d.0 = Some(t);
    d.1 = d.1.checked_add(1).unwrap_or_else(|| trap("RTV revision overflow"));
}
fn graphics_list(this: This) -> Arc<Mutex<ListState>> {
    let l = list(this);
    { let s = l.lock().unwrap(); if s.closed || s.kind != 0 { trap("invalid graphics list"); } }
    l
}
fn rtv(handle: u64) -> Option<(Arc<Mutex<(Option<Arc<Texture>>, u64)>>, Arc<Texture>, u64)> {
    let Some(Data::Rtv(h)) = data(handle as This) else { return None; };
    let (t, revision) = { let d = h.lock().unwrap(); (d.0.clone()?, d.1) };
    Some((h,t,revision))
}
extern "system" fn clear_rtv(this: This, handle: u64, color: *const f32, n: u32, rects: *const c_void) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    let Some((h,t,revision)) = rtv(handle) else { l.invalid = true; return; };
    if color.is_null() || n != 0 || !rects.is_null() || l.graphics.clear.is_some() || l.graphics.drawn { l.invalid = true; return; }
    let c = unsafe { *(color as *const [f32;4]) };
    if !c.iter().all(|v| v.is_finite()) { l.invalid = true; return; }
    l.graphics.clear = Some((h,t,revision,c));
}
extern "system" fn targets(this: This, n: u32, handles: *const u64, single: i32, depth: *const u64) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if n != 1 || handles.is_null() || single != 0 || !depth.is_null() || l.graphics.drawn { l.invalid = true; return; }
    l.graphics.target = rtv(unsafe { *handles });
    if l.graphics.target.is_none() { l.invalid = true; }
}
extern "system" fn topology(this: This, value: u32) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if value != 4 || l.graphics.drawn { l.invalid = true; } else { l.graphics.topology = true; }
}
extern "system" fn viewport(this: This, n: u32, p: *const [f32; 6]) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if n != 1 || p.is_null() || l.graphics.drawn { l.invalid = true; } else { l.graphics.viewport = Some(unsafe { *p }); }
}
extern "system" fn scissor(this: This, n: u32, p: *const [i32; 4]) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if n != 1 || p.is_null() || l.graphics.drawn { l.invalid = true; } else { l.graphics.scissor = Some(unsafe { *p }); }
}
#[repr(C)]
struct VertexView { address: u64, size: u32, stride: u32 }
extern "system" fn vertices(this: This, slot: u32, n: u32, p: *const VertexView) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if slot != 0 || n != 1 || p.is_null() || l.graphics.drawn { l.invalid = true; return; }
    let v = unsafe { &*p };
    if v.stride != 16 || v.size < 48 { l.invalid = true; return; }
    let map = objects().lock().unwrap();
    let found = map.values().find_map(|e| if let Data::Buffer(b) = &e.data {
        let base = b.ptr() as u64;
        let offset = v.address.checked_sub(base)?;
        if offset.checked_add(v.size as u64)? <= b.len() as u64 { Some((b.clone(), offset as usize)) } else { None }
    } else { None });
    drop(map);
    if found.is_none() { l.invalid = true; } else { l.graphics.vertices = found; }
}
extern "system" fn graphics_constant(this: This, slot: u32, value: u32, offset: u32) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if slot != 3 || offset >= 64 || l.graphics.drawn { l.invalid = true; return; }
    let c = &mut l.graphics.constants;
    if c.len() <= offset as usize { c.resize(offset as usize + 1, None); }
    c[offset as usize] = Some(value);
}
extern "system" fn graphics_constants(this: This, slot: u32, n: u32, values: *const u32, offset: u32) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    let end = (offset as usize).checked_add(n as usize);
    if slot != 3 || values.is_null() || end.is_none_or(|v| v > 64) || l.graphics.drawn { l.invalid = true; return; }
    let end = end.unwrap();
    let c = &mut l.graphics.constants;
    if c.len() < end { c.resize(end, None); }
    for (dst, value) in c[offset as usize..end].iter_mut().zip(unsafe { std::slice::from_raw_parts(values, n as usize) }) { *dst = Some(*value); }
}
extern "system" fn draw(this: This, count: u32, instances: u32, first: u32, instance: u32) {
    let l = graphics_list(this); let mut l = l.lock().unwrap(); let g = &mut l.graphics;
    let valid = if let (Some((ch,ct,cr,_)), Some((th,tt,tr)), Some(_), Some(_)) = (&g.clear, &g.target, &g.pipeline, &g.vertices) {
        Arc::ptr_eq(ch,th) && Arc::ptr_eq(ct,tt) && cr == tr && g.viewport == Some([0.0,0.0,ct.width as f32,ct.height as f32,0.0,1.0])
            && g.scissor == Some([0,0,ct.width as i32,ct.height as i32])
    } else { false };
    if !valid || !g.topology || g.drawn || count != 3 || instances != 1 || first != 0 || instance != 0 || g.constants.iter().any(Option::is_none) { l.invalid = true; return; }
    g.drawn = true;
}
#[repr(C)]
struct Transition { kind: u32, flags: u32, resource: This, subresource: u32, before: u32, after: u32, padding: u32 }
extern "system" fn barrier(this: This, n: u32, p: *const Transition) -> u32 {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if n != 1 || p.is_null() { l.invalid = true; return 0; }
    let b = unsafe { &*p };
    let same = match (data(b.resource), &l.graphics.clear) { (Some(Data::Texture(t)), Some((_,ct,_,_))) => Arc::ptr_eq(&t,ct), _ => false };
    if !same || !l.graphics.drawn || l.graphics.barrier || b.kind != 0 || b.flags != 0 || b.subresource != u32::MAX || b.before != 4 || b.after != 0x800 { l.invalid = true; } else { l.graphics.barrier = true; }
    0
}
#[repr(C)]
struct CopyLocation { resource: This, kind: u32, padding: u32, offset: u64, format: u32, width: u32, height: u32, depth: u32, pitch: u32 }
extern "system" fn copy_texture(this: This, dst: *const CopyLocation, x: u32, y: u32, z: u32, src: *const CopyLocation, region: *const c_void) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if dst.is_null() || src.is_null() || x != 0 || y != 0 || z != 0 || !region.is_null() || !l.graphics.barrier || l.graphics.job.is_some() { l.invalid = true; return; }
    let d = unsafe { &*dst }; let s = unsafe { &*src };
    let (Some(Data::Buffer(output)),Some(Data::Texture(texture))) = (data(d.resource),data(s.resource)) else { l.invalid = true; return; };
    let g = &l.graphics;
    let (h,ct,revision,clear) = g.clear.as_ref().unwrap();
    let row = texture.width as usize * 4;
    let end = (d.pitch as usize).checked_mul(texture.height as usize - 1)
        .and_then(|n| (d.offset as usize).checked_add(n)).and_then(|n| n.checked_add(row));
    if !Arc::ptr_eq(ct,&texture) || s.kind != 0 || s.offset as u32 != 0 || d.kind != 1 || d.offset % 512 != 0 || d.format != 28 || d.width != texture.width || d.height != texture.height || d.depth != 1 || d.pitch % 256 != 0 || (d.pitch as usize) < row || end.is_none_or(|n| n > output.len()) { l.invalid = true; return; }
    let (vertices,vertex_offset) = g.vertices.as_ref().unwrap();
    let job = GraphicsJob { pipeline: g.pipeline.as_ref().unwrap().clone(), texture, vertices: vertices.clone(), vertex_offset: *vertex_offset, constants: g.constants.iter().map(|v| v.unwrap()).collect(), clear: *clear, rtv: h.clone(), revision: *revision, output, offset: d.offset as usize, pitch: d.pitch as usize };
    l.graphics.job = Some(job);
}
fn run_graphics(c: &Config, j: &GraphicsJob) -> Result<(), String> {
    let mut s = TcpStream::connect_timeout(&c.addr, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(60))).map_err(|e| e.to_string())?;
    s.set_write_timeout(Some(Duration::from_secs(60))).map_err(|e| e.to_string())?;
    for v in [MAGIC,2,j.pipeline.vs.len() as u32,j.pipeline.ps.len() as u32,j.texture.width,j.texture.height,3,j.constants.len() as u32] { s.write_all(&v.to_le_bytes()).map_err(|e| e.to_string())?; }
    s.write_all(&c.token).map_err(|e| e.to_string())?;
    for v in j.clear { s.write_all(&v.to_bits().to_le_bytes()).map_err(|e| e.to_string())?; }
    s.write_all(&j.pipeline.vs).and_then(|_| s.write_all(&j.pipeline.ps)).map_err(|e| e.to_string())?;
    let vertices = unsafe { std::slice::from_raw_parts(j.vertices.ptr().add(j.vertex_offset),48) };
    s.write_all(vertices).map_err(|e| e.to_string())?;
    for v in &j.constants { s.write_all(&v.to_le_bytes()).map_err(|e| e.to_string())?; }
    let mut header = [0u8;16]; s.read_exact(&mut header).map_err(|e| e.to_string())?;
    let word = |n| u32::from_le_bytes(header[n..n+4].try_into().unwrap());
    let (magic,status,len,msg) = (word(0),word(4),word(8) as usize,word(12) as usize);
    let expected = j.texture.width as usize * j.texture.height as usize * 4;
    if magic != MAGIC || len > MAX_BUFFER || msg > 4096 || (status == 0 && (len != expected || msg != 0)) || (status != 0 && len != 0) { return Err("invalid proxy response".into()); }
    let mut bytes = vec![0;len]; let mut message = vec![0;msg];
    s.read_exact(&mut bytes).and_then(|_| s.read_exact(&mut message)).map_err(|e| e.to_string())?;
    let message = String::from_utf8(message).map_err(|e| e.to_string())?;
    if status != 0 { return Err(format!("proxy status {status}: {message}")); }
    let row = j.texture.width as usize * 4;
    for y in 0..j.texture.height as usize { unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr().add(y*row),j.output.ptr().add(j.offset+y*j.pitch),row); } }
    tracing::info!("Wine Metal proxy connected");
    Ok(())
}
