//! Synchronous Wine D3D12.X bridge with copied, ordered graphics submissions.
use std::{
    cell::UnsafeCell,
    collections::{HashMap, VecDeque},
    ffi::c_void,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{Arc, Condvar, LazyLock, Mutex, atomic::{AtomicU64, Ordering}},
    time::Duration,
};
use crate::object::{guid, HRESULT, Iid, Object, Table, This, E_INVALIDARG, E_NOINTERFACE, S_OK};
use crate::wine_transport::{self, Batch, Resource, ResourceKind, Command, Draw, CopyTexture, Blend, Pipeline, PipelineDepth, Stencil, Index, VertexInput};

static NEXT_RESOURCE: AtomicU64 = AtomicU64::new(1);
fn resource_id() -> u64 {
    let id = NEXT_RESOURCE.fetch_add(1, Ordering::Relaxed);
    if id == 0 { trap("resource identity exhausted"); }
    id
}

const FAIL: HRESULT = 0x80004005u32 as i32;
const MAGIC: u32 = 0x31474d56;
const MAX_SHADER: usize = 4 * 1024 * 1024;
const MAX_BUFFER: usize = 16 * 1024 * 1024;
const MAX_COMPUTE_PAYLOAD: usize = 64 * 1024 * 1024;
const UNKNOWN: Iid = guid(0, 0, 0, [0xc0, 0, 0, 0, 0, 0, 0, 0x46]);
const DEVICE11: Iid = guid(0x5405c344, 0xd457, 0x444e, [0xb4, 0xdd, 0x23, 0x66, 0xe4, 0x5a, 0xee, 0x39]);
const HEAP: Iid = guid(0x8efb471d, 0x616c, 0x4f49, [0x90, 0xf7, 0x12, 0x7b, 0xb7, 0x63, 0xfa, 0x51]);
const RESOURCE: Iid = guid(0x696442be, 0xa72e, 0x4059, [0xbc, 0x79, 0x5b, 0x5c, 0x98, 0x04, 0x0f, 0xad]);
const PIPELINE: Iid = guid(0x765a30f3, 0xf624, 0x4c6f, [0xa8, 0x28, 0xac, 0xe9, 0x48, 0x62, 0x24, 0x45]);
const ALLOCATOR: Iid = guid(0x6102dee4, 0xaf59, 0x4b09, [0xb9, 0x99, 0xb4, 0x4d, 0x73, 0xf0, 0x9b, 0x24]);
const LIST: Iid = guid(0x7116d91c, 0xe7e4, 0x47ce, [0xb8, 0xc6, 0xec, 0x81, 0x68, 0xf4, 0x37, 0xe5]);
const GRAPHICS_LIST: Iid = guid(0x5b160d0f, 0xac1b, 0x4185, [0x8b, 0xa8, 0xb3, 0xae, 0x42, 0xa5, 0xa4, 0x55]);
const NEWER_LIST_IIDS: [Iid; 6] = [
    guid(0x553103fb, 0x1fe7, 0x4557, [0xbb, 0x38, 0x94, 0x6d, 0x7d, 0x0e, 0x7c, 0xa7]),
    guid(0x38c3e585, 0xff17, 0x412c, [0x91, 0x50, 0x4f, 0xc6, 0xf9, 0xd7, 0x2a, 0x28]),
    guid(0x6fda83a7, 0xb84c, 0x4e38, [0x9a, 0xc8, 0xc7, 0xbd, 0x22, 0x01, 0x6b, 0x3d]),
    guid(0x8754318e, 0xd3a9, 0x4541, [0x98, 0xcf, 0x64, 0x5b, 0x50, 0xdc, 0x48, 0x74]),
    guid(0x55050859, 0x4024, 0x474c, [0x98, 0xcf, 0x64, 0x5b, 0x51, 0xdc, 0x48, 0x74]),
    guid(0xc3827890, 0x4024, 0x474c, [0x98, 0xcf, 0x64, 0x5b, 0x51, 0xdc, 0x48, 0x74]),
];
const QUEUE: Iid = guid(0x0ec870a6, 0x5d7e, 0x4c22, [0x8c, 0xfc, 0x5b, 0xaa, 0xe0, 0x76, 0x16, 0xed]);

#[derive(Clone)]
struct Config {
    addr: SocketAddr,
    token: [u8; 16],
    newer: bool,
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
    Ok(Config { addr, token, newer: false })
}

// These objects keep their COM identity in a registry; recorded jobs retain
// their resources and bytecode independently of guest Release calls.
#[repr(C)]
struct Header {
    object: Object,
    inline: [u64; 180],
    dxgi: Object,
}
struct Entry {
    refs: u32,
    data: Data,
}
#[derive(Clone)]
enum Data {
    Device(Arc<Config>),
    Heap(Arc<Mutex<BufferHeap>>, TextureHeap),
    Sampler(Arc<Mutex<Option<bool>>>),
    Buffer(Arc<Buffer>),
    Pipeline(Arc<Vec<u8>>),
    Allocator(u32),
    List(Arc<Mutex<ListState>>, bool),
    Queue(Arc<ProxyQueue>),
    Fence(Arc<ProxyFence>),
    Texture(Arc<Texture>),
    Rtv(Arc<Mutex<(Option<Arc<Texture>>, u64)>>),
    Dsv(Arc<Mutex<(Option<Arc<Texture>>, u64)>>),
    GraphicsPipeline(Arc<GraphicsPipeline>),
}
struct Buffer {
    id: u64,
    state: Mutex<u32>,
    allow_uav: bool,
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
#[derive(Clone)]
struct BufferView { buffer: Arc<Buffer>, offset: u32, length: u32 }
#[derive(Clone)]
enum BufferDescriptor { Srv(BufferView), Uav(BufferView) }
impl BufferDescriptor {
    fn view(&self) -> &BufferView {
        match self { Self::Srv(view) | Self::Uav(view) => view }
    }
}
struct BufferHeap { entries: Vec<Option<BufferDescriptor>>, generation: u64 }
type ComputeHeap = Arc<Mutex<BufferHeap>>;

fn buffer_handle(handle: u64) -> Option<(ComputeHeap, usize)> {
    let map = objects().lock().unwrap();
    map.iter().find_map(|(&base, entry)| {
        let Data::Heap(heap, _) = &entry.data else { return None; };
        let index = handle.checked_sub(base as u64)?;
        if index < heap.lock().unwrap().entries.len() as u64 {
            Some((heap.clone(), index as usize))
        } else { None }
    })
}
fn raw_view(buffer: Arc<Buffer>, first: u64, elements: u32) -> Option<BufferView> {
    let offset = first.checked_mul(4)?;
    let length = u64::from(elements).checked_mul(4)?;
    if length == 0 || offset.checked_add(length)? > buffer.len() as u64 { return None; }
    Some(BufferView { buffer, offset: u32::try_from(offset).ok()?, length: u32::try_from(length).ok()? })
}
extern "system" fn descriptor_increment(this: This, kind: u32) -> u32 {
    if device(this).is_none() || kind > 3 { trap("invalid descriptor heap type"); }
    1
}
fn write_buffer_descriptor(heap: &ComputeHeap, index: usize, descriptor: BufferDescriptor) {
    let mut heap = heap.lock().unwrap();
    heap.entries[index] = Some(descriptor);
    heap.generation = heap.generation.checked_add(1).expect("descriptor generation overflow");
}
// Guest access to mapped memory and queue submission must be synchronized by
// the guest, as with an ordinary persistently mapped D3D resource.
unsafe impl Send for Buffer {}
unsafe impl Sync for Buffer {}
struct Texture { id: u64, width: u32, height: u32, format: u32, mips: u32, state: Mutex<u32>, bytes: Mutex<Vec<u8>>, tiled: Option<(Buffer, victory_formats::texture::Texture)> }
#[derive(Clone)]
struct Srv { texture: Arc<Texture>, first_mip: u32, mip_count: u32 }
type TextureHeap = Arc<Mutex<Option<Srv>>>;
struct GraphicsPipeline { id: u64, vs: Vec<u8>, ps: Vec<u8>, stride: u32, inputs: Vec<VertexInput>, legacy: bool, depth: Option<PipelineDepth>, blend: Blend }
type Rtv = (Arc<Mutex<(Option<Arc<Texture>>, u64)>>, Arc<Texture>, u64);
#[derive(Clone)]
enum GraphicsResource { Buffer(Arc<Buffer>), Texture(Arc<Texture>) }
impl GraphicsResource {
    fn id(&self) -> u64 { match self { Self::Buffer(b) => b.id, Self::Texture(t) => t.id } }
    fn snapshot(&self) -> Result<Resource, String> {
        Ok(match self {
            Self::Buffer(b) => Resource { id: b.id, state: *b.state.lock().unwrap(), kind: ResourceKind::Buffer,
                bytes: unsafe { std::slice::from_raw_parts(b.ptr(), b.len()) }.to_vec() },
            Self::Texture(t) => {
                let bytes = if let Some((memory, layout)) = &t.tiled {
                    let source = unsafe { std::slice::from_raw_parts(memory.ptr(), memory.len()) };
                    let mut bytes = vec![0; wine_transport::texture_size(t.format, t.width, t.height, t.mips).ok_or("invalid texture size")?];
                    for mip in 0..t.mips {
                        let (width, height, offset, size) = wine_transport::mip_layout(t.format, t.width, t.height, t.mips, mip).ok_or("invalid mip")?;
                        let (row, rows) = wine_transport::block_geometry(t.format, width, height).ok_or("invalid texture format")?;
                        let end = offset.checked_add(size).ok_or("invalid mip size")?;
                        layout.planes.first().ok_or("missing texture plane")?.layout.detile(source, mip, 0, bytes.get_mut(offset..end).ok_or("invalid mip range")?, row, row.checked_mul(rows).ok_or("invalid mip size")?).map_err(|e| format!("texture detile failed: {e:?}"))?;
                    }
                    bytes
                } else { t.bytes.lock().unwrap().clone() };
                Resource { id: t.id, state: *t.state.lock().unwrap(),
                    kind: ResourceKind::Texture { width: t.width, height: t.height, format: t.format, mips: t.mips }, bytes }
            }
        })
    }
}
#[derive(Clone)]
struct GraphicsJob {
    command: Command, resources: Vec<GraphicsResource>, descriptor: Option<Rtv>,
    depth_descriptor: Option<Rtv>,
    pipeline: Option<Arc<GraphicsPipeline>>,
    texture: Option<(TextureHeap, Srv)>,
    sampler: Option<(Arc<Mutex<Option<bool>>>, bool)>,
}
#[derive(Default)]
struct GraphicsState {
    pipeline: Option<Arc<GraphicsPipeline>>,
    target: Option<Rtv>,
    depth: Option<Rtv>,
    stencil_ref: u32,
    viewport: Option<[f32; 6]>, scissor: Option<[i32; 4]>, topology: bool,
    vertices: [Option<(Arc<Buffer>, usize, u32, u32)>; 16], constants: Vec<Option<u32>>,
    index: Option<(Arc<Buffer>, usize, u32, u32)>,
    heaps: Vec<usize>, texture: Option<(u32, TextureHeap, Srv)>,
    sampler: Option<(u32, Arc<Mutex<Option<bool>>>, bool)>, constant_slot: Option<u32>,
    jobs: Vec<GraphicsJob>,
}
#[derive(Clone)]
struct Job {
    shader: Arc<Vec<u8>>,
    heap: ComputeHeap,
    generation: u64,
    descriptors: Vec<BufferDescriptor>,
    table_slot: u32,
    table_base: u32,
    constants: Vec<u32>,
    cbv: Option<RootCbv>,
    groups: [u32; 3],
}

#[derive(Clone)]
struct RootCbv {
    slot: u32,
    buffer: Arc<Buffer>,
    offset: usize,
}
#[derive(Default)]
struct ListState {
    closed: bool,
    invalid: bool,
    pipeline: Option<Arc<Vec<u8>>>,
    heap: Option<(usize, ComputeHeap)>,
    table: Option<(u32, u32)>,
    constants: Vec<Option<u32>>,
    cbv: Option<RootCbv>,
    jobs: Vec<Job>,
    kind: u32,
    graphics: GraphicsState,
}

struct EventHandle(usize);
impl Drop for EventHandle {
    fn drop(&mut self) {
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0 as *mut c_void); }
    }
}

struct FenceState {
    value: u64,
    events: Vec<(u64, EventHandle)>,
}
struct ProxyFence {
    config: Arc<Config>,
    state: Mutex<FenceState>,
    changed: Condvar,
}
impl ProxyFence {
    fn value(&self) -> u64 {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).value
    }
    fn wait(&self, value: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        while state.value < value {
            state = self.changed.wait(state).unwrap_or_else(|e| e.into_inner());
        }
    }
    fn signal(&self, value: u64) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.value = value;
        let mut index = 0;
        while index < state.events.len() {
            if state.events[index].0 <= value {
                let (_, event) = state.events.swap_remove(index);
                unsafe { windows_sys::Win32::System::Threading::SetEvent(event.0 as *mut c_void); }
            } else {
                index += 1;
            }
        }
        self.changed.notify_all();
    }
}

#[derive(Default)]
struct QueueState {
    active: bool,
    pending: VecDeque<QueueOp>,
}
struct ProxyQueue {
    config: Arc<Config>,
    kind: u32,
    state: Mutex<QueueState>,
}
enum QueueOp {
    Wait(Arc<ProxyFence>, u64),
    Signal(Arc<ProxyFence>, u64),
    Compute(Vec<Job>),
    Graphics(Vec<GraphicsJob>),
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
fn supports(data: &Data, expected: Iid, iid: &Iid) -> bool {
    *iid == expected || *iid == UNKNOWN || (expected == LIST && *iid == GRAPHICS_LIST)
        || (matches!(data, Data::Device(_)) && crate::dxgi::DEVICE_IIDS.contains(iid))
        || (matches!(data, Data::Device(c) if c.newer) && *iid == DEVICE11)
        || (matches!(data, Data::List(_, true)) && NEWER_LIST_IIDS.contains(iid))
        || (matches!(data, Data::Fence(_)) && crate::fence::FENCE_IIDS.contains(iid))
}
fn new(data: Data, table: &'static Table<160>, iid: *const Iid, expected: Iid, out: *mut This) -> HRESULT {
    if iid.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    unsafe {
        *out = std::ptr::null();
        if !supports(&data, expected, &*iid) {
            return E_NOINTERFACE;
        }
    }
    let mut header = Box::new(Header { object: Object::new(table), inline: [0; 180], dxgi: Object::new(&DXGI_DEVICE_TABLE) });
    match &data {
        Data::Device(_) => header.inline[1] = 1,
        Data::Queue(queue) => header.inline[0] = u64::from(queue.kind),
        Data::List(state, _) => header.inline[0] = u64::from(state.lock().unwrap().kind),
        _ => {}
    }
    let ptr = Box::into_raw(header) as This;
    objects().lock().unwrap().insert(ptr as usize, Entry { refs: 1, data });
    unsafe { *out = ptr; }
    S_OK
}
// The DXGI interface is embedded in its device allocation and shares the
// controlling IUnknown and reference count. No child may outlive that count.
fn object_key(map: &HashMap<usize, Entry>, this: This) -> Option<usize> {
    let address = this as usize;
    if map.contains_key(&address) { return Some(address); }
    let device = address.checked_sub(std::mem::offset_of!(Header, dxgi))?;
    map.get(&device).filter(|entry| matches!(entry.data, Data::Device(_))).map(|_| device)
}
extern "system" fn query(this: This, iid: *const Iid, out: *mut This) -> HRESULT {
    if iid.is_null() || out.is_null() {
        return E_INVALIDARG;
    }
    unsafe { *out = std::ptr::null(); }
    let mut map = objects().lock().unwrap();
    let Some(key) = object_key(&map, this) else { return E_INVALIDARG; };
    let Some(entry) = map.get_mut(&key) else {
        return E_INVALIDARG;
    };
    let expected = match &entry.data {
        Data::Device(_) => crate::device::IID,
        Data::Heap(..) | Data::Sampler(_) => HEAP,
        Data::Buffer(_) => RESOURCE,
        Data::Pipeline(_) => PIPELINE,
        Data::Allocator(_) => ALLOCATOR,
        Data::List(..) => LIST,
        Data::Queue(_) => QUEUE,
        Data::Fence(_) => crate::fence::FENCE_IIDS[0],
        Data::Texture(_) => RESOURCE,
        Data::Rtv(_) | Data::Dsv(_) => HEAP,
        Data::GraphicsPipeline(_) => PIPELINE,
    };
    if unsafe { !supports(&entry.data, expected, &*iid) } {
        return E_NOINTERFACE;
    }
    entry.refs = entry.refs.checked_add(1).unwrap_or_else(|| trap("reference overflow"));
    unsafe {
        *out = if matches!(entry.data, Data::Device(_)) && crate::dxgi::DEVICE_IIDS.contains(&*iid) {
            (key + std::mem::offset_of!(Header, dxgi)) as This
        } else {
            key as This
        };
    }
    S_OK
}
extern "system" fn add_ref(this: This) -> u32 {
    let mut m = objects().lock().unwrap();
    let key = object_key(&m, this).unwrap_or_else(|| trap("invalid AddRef"));
    let e = m.get_mut(&key).unwrap_or_else(|| trap("invalid AddRef"));
    e.refs = e.refs.checked_add(1).unwrap_or_else(|| trap("reference overflow"));
    e.refs
}
extern "system" fn release(this: This) -> u32 {
    let mut m = objects().lock().unwrap();
    let key = object_key(&m, this).unwrap_or_else(|| trap("invalid Release"));
    let e = m.get_mut(&key).unwrap_or_else(|| trap("invalid Release"));
    e.refs -= 1;
    let n = e.refs;
    if n == 0 {
        m.remove(&key);
        unsafe { drop(Box::from_raw(key as *mut Header)); }
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
static DXGI_DEVICE_TABLE: Table<160> = {
    let mut t = base();
    t[6] = crate::dxgi::device_parent as *const ();
    t[7] = crate::dxgi::device_adapter as *const ();
    Table(t)
};
static DEVICE_TABLE: Table<160> = {
    let mut t = base();
    t[8] = create_queue as *const ();
    t[9] = create_allocator as *const ();
    t[11] = create_pipeline as *const ();
    t[12] = create_list as *const ();
    t[14] = create_heap as *const ();
    t[15] = descriptor_increment as *const ();
    t[20] = create_rtv as *const ();
    t[21] = create_dsv as *const ();
    t[18] = create_srv as *const ();
    t[22] = create_sampler as *const ();
    t[86] = create_graphics_pipeline as *const ();
    t[19] = create_uav as *const ();
    t[48] = create_queue_x as *const ();
    t[73] = create_list_x as *const ();
    t[27] = create_resource as *const ();
    t[36] = create_fence as *const ();
    t[38] = copyable_footprints as *const ();
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
static FENCE_TABLE: Table<160> = {
    let mut t = base();
    t[8] = fence_completed as *const ();
    t[9] = fence_event as *const ();
    t[10] = fence_signal as *const ();
    Table(t)
};
static LIST_TABLE: Table<160> = {
    let mut t = base();
    t[8] = list_type as *const ();
    t[9] = close as *const ();
    t[12] = draw as *const ();
    t[13] = draw_indexed as *const ();
    t[15] = copy_buffer as *const ();
    t[16] = copy_texture as *const ();
    t[20] = topology as *const ();
    t[21] = viewport as *const ();
    t[22] = scissor as *const ();
    t[24] = set_stencil_ref as *const ();
    t[26] = barrier as *const ();
    t[38] = graphics_constant as *const ();
    t[51] = clear_dsv as *const ();
    t[40] = graphics_constants as *const ();
    t[47] = indices as *const ();
    t[48] = vertices as *const ();
    t[50] = targets as *const ();
    t[52] = clear_rtv as *const ();
    t[14] = dispatch as *const ();
    t[25] = set_pipeline as *const ();
    t[32] = set_heaps as *const ();
    t[35] = set_table as *const ();
    t[36] = graphics_table as *const ();
    t[39] = set_constants as *const ();
    t[41] = set_cbv as *const ();
    Table(t)
};
static QUEUE_TABLE: Table<160> = {
    let mut t = base();
    t[10] = execute as *const ();
    t[14] = queue_signal as *const ();
    t[15] = queue_wait as *const ();
    t[30] = present as *const ();
    t[33] = execute_x as *const ();
    Table(t)
};
static NEWER_QUEUE_TABLE: Table<160> = {
    let mut t = base();
    t[10] = execute as *const ();
    t[16] = queue_signal as *const ();
    t[17] = queue_wait as *const ();
    t[32] = present as *const ();
    t[35] = execute_x as *const ();
    Table(t)
};

fn pin_proxy() -> bool {
    static PINNED: LazyLock<bool> = LazyLock::new(|| {
        use windows_sys::Win32::System::LibraryLoader::{
            GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
        };
        let mut module = std::ptr::null_mut();
        // The proxy's objects and deferred queue worker contain code in this DLL.
        unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
                create as *const () as *const u16,
                &mut module,
            ) != 0
        }
    });
    *PINNED
}

pub(super) fn create(iid: &Iid, out: *mut This) -> HRESULT {
    unsafe { *out = std::ptr::null(); }
    match config() {
        Ok(mut c) if *iid == crate::device::IID || *iid == DEVICE11 => {
            if !pin_proxy() { return FAIL; }
            c.newer = *iid == DEVICE11;
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
    if device(this).is_none() || d.count == 0 || d.count > 16 || d.nodes != 0 {
        return E_INVALIDARG;
    }
    if d.kind == 2 && d.flags == 0 && d.count == 1 {
        return new(Data::Rtv(Arc::new(Mutex::new((None, 0)))), &HEAP_TABLE, iid, HEAP, out);
    }
    if d.kind == 3 && d.flags == 0 && d.count == 1 {
        return new(Data::Dsv(Arc::new(Mutex::new((None, 0)))), &HEAP_TABLE, iid, HEAP, out);
    }
    if d.kind == 1 && d.flags == 1 && d.count == 1 {
        return new(Data::Sampler(Arc::new(Mutex::new(None))), &HEAP_TABLE, iid, HEAP, out);
    }
    if d.kind != 0 || d.flags != 1 { return E_INVALIDARG; }
    new(Data::Heap(Arc::new(Mutex::new(BufferHeap { entries: vec![None; d.count as usize], generation: 0 })), Arc::new(Mutex::new(None))), &HEAP_TABLE, iid, HEAP, out)
}
extern "system" fn heap_handle(this: This) -> u64 {
    if !matches!(data(this), Some(Data::Heap(..)) | Some(Data::Sampler(_)) | Some(Data::Rtv(_)) | Some(Data::Dsv(_))) {
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
const _: () = assert!(std::mem::size_of::<ResourceDesc>() == std::mem::size_of::<crate::resource::ResourceDesc>());

// Delegate the proven upstream layout after checking the proxy's supported
// resource subset and every arithmetic bound used by the upstream helper.
extern "system" fn copyable_footprints(
    this: This, desc: *const ResourceDesc, first: u32, count: u32, base: u64,
    layouts: *mut crate::resource::PlacedFootprint, rows: *mut u32, row_sizes: *mut u64, total: *mut u64,
) {
    if !total.is_null() { unsafe { *total = u64::MAX; } }
    let Some(r) = (unsafe { desc.as_ref() }) else { return; };
    if device(this).is_none() || r.alignment != 0 || r.width == 0 || r.depth != 1 || r.mips == 0
        || r.samples != 1 || r.quality != 0 || first.checked_add(count).is_none_or(|end| end > u32::from(r.mips)) {
        return;
    }
    if r.dimension == 1 {
        if r.width > MAX_BUFFER as u64 || r.height != 1 || r.mips != 1 || r.format != 0
            || r.layout != 1 || !matches!(r.flags, 0 | 4) { return; }
    } else if r.dimension == 3 {
        let tiled = matches!(r.layout, 0x105 | 0x10a);
        if r.width > u32::MAX as u64 || wine_transport::texture_size(r.format, r.width as u32, r.height, u32::from(r.mips)).is_none_or(|n| n > MAX_BUFFER)
            || !matches!(r.format, 28 | 40 | 71 | 72 | 77 | 78) || !matches!(r.layout, 0 | 1 | 0x105 | 0x10a)
            || (tiled && (r.flags != 0 || r.format != 28))
            || (r.mips != 1 && (r.flags != 0 || (r.layout != 0 && !tiled)))
            || (matches!(r.format, 71 | 72 | 77 | 78) && (r.width % 4 != 0 || r.height % 4 != 0 || (r.layout != 0 && !tiled) || r.flags != 0))
            || (r.format == 28 && !matches!(r.flags, 0 | 1))
            || (r.format == 40 && (r.mips != 1 || r.flags != 2)) { return; }
    } else { return; }
    let mut end = base;
    for mip in first..first + count {
        let (row, rows) = if r.dimension == 1 {
            (r.width as usize, 1)
        } else {
            let Some((w, h, _, _)) = wine_transport::mip_layout(r.format, r.width as u32, r.height, u32::from(r.mips), mip) else { return; };
            let Some(v) = wine_transport::block_geometry(r.format, w, h) else { return; };
            v
        };
        let Some(offset) = end.checked_add(511).map(|v| v & !511) else { return; };
        let Some(pitch) = (row as u64).checked_add(255).map(|v| v & !255).filter(|&v| v <= u32::MAX as u64) else { return; };
        let Some(next) = pitch.checked_mul((rows - 1) as u64).and_then(|n| offset.checked_add(n)).and_then(|n| n.checked_add(row as u64)) else { return; };
        end = next;
    }
    // SAFETY: both descriptions are repr(C), have identical D3D12 fields,
    // and optional outputs follow the upstream method's `count` contract.
    crate::resource::copyable_footprints(this, desc.cast(), first, count, base, layouts, rows, row_sizes, total);
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
    if !out.is_null() { unsafe { *out = std::ptr::null(); } }
    let (Some(h), Some(r)) = (unsafe { hp.as_ref() }, unsafe { rd.as_ref() }) else {
        return E_INVALIDARG;
    };
    if device(this).is_none() || h.kind != 1 || h.cpu != 0 || h.pool != 0
        || h.creation > 1 || h.visible > 1 || flags != 0 || !clear.is_null()
        || r.alignment != 0 || r.width == 0 || r.depth != 1
        || r.samples != 1 || r.quality != 0 {
        return E_INVALIDARG;
    }
    if r.dimension == 3 {
        if r.width > u32::MAX as u64 { return E_INVALIDARG; }
        let Some(size) = wine_transport::texture_size(r.format, r.width as u32, r.height, u32::from(r.mips)) else { return E_INVALIDARG; };
        let bc = matches!(r.format, 71 | 72 | 77 | 78);
        let tiled = matches!(r.layout, 0x105 | 0x10a);
        if size > MAX_BUFFER || !matches!(r.format, 20 | 28 | 40 | 71 | 72 | 77 | 78) || !matches!(r.layout, 0 | 1 | 0x105 | 0x10a)
            || (tiled && (r.flags != 0 || r.format != 28 || !matches!(state, 0 | 0x400 | 0x800 | 0x80)))
            || (r.mips != 1 && (r.flags != 0 || (r.layout != 0 && !tiled)))
            || (bc && (r.width % 4 != 0 || r.height % 4 != 0 || (r.layout != 0 && !tiled) || r.flags != 0 || !matches!(state, 0 | 0x400 | 0x800 | 0x80)))
            || (r.format == 28 && (!matches!(r.flags, 0 | 1) || !matches!(state, 0 | 4 | 0x400 | 0x800 | 0x80)))
            || (r.format == 40 && (r.mips != 1 || r.flags != 2 || !matches!(state, 0 | 0x10 | 0x800)))
            || (r.format == 20 && (r.mips != 1 || r.flags != 2 || r.layout != 0 || !matches!(state, 0 | 0x10 | 0x400 | 0x800))) {
            return E_INVALIDARG;
        }
        let tiled = if tiled {
            // SAFETY: the two repr(C) descriptions have identical fields.
            let desc = unsafe { &*(rd.cast::<crate::resource::ResourceDesc>()) };
            let Some(layout) = crate::resource::texture_layout(desc) else { return E_INVALIDARG; };
            let Ok(len) = usize::try_from(layout.size) else { return E_INVALIDARG; };
            if len == 0 || len > MAX_BUFFER || layout.planes.len() != 1 { return E_INVALIDARG; }
            let bytes = (0..len).map(|_| UnsafeCell::new(0)).collect::<Vec<_>>().into_boxed_slice();
            Some((Buffer { id: 0, state: Mutex::new(state), allow_uav: false, bytes }, layout))
        } else { None };
        let bytes = if tiled.is_some() { Vec::new() } else { vec![0; size] };
        return new(Data::Texture(Arc::new(Texture { id: resource_id(), width: r.width as u32, height: r.height, format: r.format, mips: u32::from(r.mips), state: Mutex::new(state), bytes: Mutex::new(bytes), tiled })), &BUFFER_TABLE, iid, RESOURCE, out);
    }
    if r.dimension != 1 || r.mips != 1 || r.width > MAX_BUFFER as u64 || r.height != 1 || r.format != 0
        || r.layout != 1 || !matches!(r.flags, 0 | 4) || !matches!(state, 0 | 1 | 8 | 0x40 | 0x400 | 0x800) {
        return E_INVALIDARG;
    }
    let bytes = (0..r.width).map(|_| UnsafeCell::new(0)).collect::<Vec<_>>().into_boxed_slice();
    new(Data::Buffer(Arc::new(Buffer { id: resource_id(), state: Mutex::new(state), allow_uav: r.flags & 4 != 0, bytes })), &BUFFER_TABLE, iid, RESOURCE, out)
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
    let backing = match data(this) {
        Some(Data::Buffer(b)) => {
            if let Some(r) = unsafe { range.as_ref() } {
                if r.begin > r.end || r.end > b.len() { return E_INVALIDARG; }
            }
            b.ptr()
        }
        Some(Data::Texture(t)) => {
            let Some((b, _)) = &t.tiled else { return E_INVALIDARG; };
            if let Some(r) = unsafe { range.as_ref() } {
                if r.begin > r.end || r.end > b.len() { return E_INVALIDARG; }
            }
            b.ptr()
        }
        _ => return E_INVALIDARG,
    };
    unsafe { *out = backing.cast(); }
    S_OK
}
extern "system" fn unmap(this: This, sub: u32, _range: *const c_void) {
    if sub != 0 || !match data(this) { Some(Data::Buffer(_)) => true, Some(Data::Texture(t)) => t.tiled.is_some(), _ => false } {
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
    let (Some(Data::Buffer(b)), Some((h, index)), Some(d)) =
        (data(res), buffer_handle(handle), unsafe { desc.as_ref() })
    else { trap("invalid UAV arguments"); };
    if device(this).is_none() || !b.allow_uav || !counter.is_null() || d.format != 39 || d.dimension != 1
        || d.stride != 0 || d.counter != 0 || d.flags != 1
    { trap("unsupported UAV view"); }
    let Some(view) = raw_view(b, d.first, d.elements) else { trap("invalid UAV range"); };
    write_buffer_descriptor(&h, index, BufferDescriptor::Uav(view));
}
#[repr(C)]
struct BufferSrvDesc {
    format: u32, dimension: u32, mapping: u32, padding: std::mem::MaybeUninit<u32>,
    first: u64, elements: u32, stride: u32, flags: u32,
}
extern "system" fn create_srv(this: This, res: This, desc: *const c_void, handle: u64) {
    if let Some(Data::Buffer(b)) = data(res) {
        let (Some((h, index)), Some(d)) = (buffer_handle(handle), unsafe { desc.cast::<BufferSrvDesc>().as_ref() })
            else { trap("invalid buffer SRV"); };
        if device(this).is_none()
            || d.format != 39 || d.dimension != 1 || d.mapping != 5768 || d.stride != 0 || d.flags != 1
        { trap("unsupported buffer SRV"); }
        let Some(view) = raw_view(b, d.first, d.elements) else { trap("invalid SRV range"); };
        write_buffer_descriptor(&h, index, BufferDescriptor::Srv(view));
        return;
    }
    let (Some(Data::Texture(t)), Some(Data::Heap(_, h))) = (data(res), data(handle as This)) else { trap("invalid SRV"); };
    if device(this).is_none() || !matches!(t.format, 28 | 71 | 72 | 77 | 78) { trap("unsupported SRV"); }
    let (first_mip, mip_count) = if desc.is_null() {
        (0, t.mips)
    } else {
        // D3D12_SHADER_RESOURCE_VIEW_DESC's Texture2D union starts at byte
        // 16. Read only defined members, not the uninitialized alignment gap.
        let p = desc.cast::<u32>();
        let (format, dimension, mapping, first, levels, plane, lod) = unsafe {
            (p.read(), p.add(1).read(), p.add(2).read(), p.add(4).read(),
             p.add(5).read(), p.add(6).read(), f32::from_bits(p.add(7).read()))
        };
        let Some(remaining) = t.mips.checked_sub(first).filter(|&n| n != 0) else { trap("invalid SRV mip range"); };
        let count = if levels == u32::MAX { remaining } else { levels };
        if format != t.format || dimension != 4 || mapping != 5768 || plane != 0 || lod != 0.0
            || count == 0 || count > remaining {
            trap("unsupported SRV");
        }
        (first, count)
    };
    *h.lock().unwrap() = Some(Srv { texture: t, first_mip, mip_count });
}
extern "system" fn create_sampler(this: This, desc: *const [u32; 13], handle: u64) {
    let (Some(Data::Sampler(h)), Some(d)) = (data(handle as This), unsafe { desc.as_ref() }) else { trap("invalid sampler"); };
    if device(this).is_none() || !matches!(d[0], 0 | 0x15) || d[1..] != [3,3,3,0,1,8,0,0,0,0,0,0] { trap("unsupported sampler"); }
    *h.lock().unwrap() = Some(d[0] == 0x15);
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
    let Some(c) = device(this) else { return E_INVALIDARG; };
    if nodes != 0 || (kind != 0 && kind != 2) || !matches!(data(alloc), Some(Data::Allocator(k)) if k == kind) {
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
    new(Data::List(Arc::new(Mutex::new(state)), c.newer), &LIST_TABLE, iid, LIST, out,
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
    let table = if c.newer { &NEWER_QUEUE_TABLE } else { &QUEUE_TABLE };
    new(Data::Queue(Arc::new(ProxyQueue { config: c, kind: d.kind, state: Mutex::new(QueueState::default()) })), table, iid, QUEUE, out)
}

#[repr(C)]
struct QueueDescX {
    kind: u32,
    flags: u32,
    engine_or_pipe: u32,
    queue: u32,
}
extern "system" fn create_queue_x(this: This, d: *const QueueDescX, iid: *const Iid, out: *mut This) -> HRESULT {
    if !out.is_null() { unsafe { *out = std::ptr::null(); } }
    let (Some(c), Some(d)) = (device(this), unsafe { d.as_ref() }) else {
        return E_INVALIDARG;
    };
    if d.flags != 0 || !matches!((d.kind, d.engine_or_pipe, d.queue), (0, 0, 0) | (2, 1, 0)) {
        return E_INVALIDARG;
    }
    let table = if c.newer { &NEWER_QUEUE_TABLE } else { &QUEUE_TABLE };
    new(Data::Queue(Arc::new(ProxyQueue { config: c, kind: d.kind, state: Mutex::new(QueueState::default()) })), table, iid, QUEUE, out)
}

extern "system" fn create_fence(this: This, initial: u64, flags: u32, iid: *const Iid, out: *mut This) -> HRESULT {
    if !out.is_null() { unsafe { *out = std::ptr::null(); } }
    let Some(config) = device(this) else { return E_INVALIDARG; };
    if flags != 0 || initial == u64::MAX { return E_INVALIDARG; }
    let fence = ProxyFence {
        config,
        state: Mutex::new(FenceState { value: initial, events: Vec::new() }),
        changed: Condvar::new(),
    };
    new(Data::Fence(Arc::new(fence)), &FENCE_TABLE, iid, crate::fence::FENCE_IIDS[0], out)
}

extern "system" fn fence_completed(this: This) -> u64 {
    match data(this) { Some(Data::Fence(fence)) => fence.value(), _ => u64::MAX }
}

extern "system" fn fence_signal(this: This, value: u64) -> HRESULT {
    let Some(Data::Fence(fence)) = data(this) else { return E_INVALIDARG; };
    if value == u64::MAX { return E_INVALIDARG; }
    fence.signal(value);
    S_OK
}

extern "system" fn fence_event(this: This, value: u64, event: *mut c_void) -> HRESULT {
    use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetEvent};
    let Some(Data::Fence(fence)) = data(this) else { return E_INVALIDARG; };
    if value == u64::MAX { return E_INVALIDARG; }
    if event.is_null() {
        fence.wait(value);
        return S_OK;
    }
    let mut copy = std::ptr::null_mut();
    let process = unsafe { GetCurrentProcess() };
    if unsafe { DuplicateHandle(process, event, process, &mut copy, 0, 0, DUPLICATE_SAME_ACCESS) } == 0 {
        return E_INVALIDARG;
    }
    let copy = EventHandle(copy as usize);
    let mut state = fence.state.lock().unwrap_or_else(|e| e.into_inner());
    if state.value >= value {
        if unsafe { SetEvent(copy.0 as *mut c_void) } == 0 { return E_INVALIDARG; }
    } else {
        state.events.push((value, copy));
    }
    S_OK
}

extern "system" fn queue_signal(this: This, fence: This, value: u64) -> HRESULT {
    let (Some(Data::Queue(queue)), Some(Data::Fence(fence))) = (data(this), data(fence)) else { return E_INVALIDARG; };
    if value == u64::MAX || !Arc::ptr_eq(&queue.config, &fence.config) { return E_INVALIDARG; }
    submit_queue(&queue, QueueOp::Signal(fence, value))
}

extern "system" fn queue_wait(this: This, fence: This, value: u64) -> HRESULT {
    let (Some(Data::Queue(queue)), Some(Data::Fence(fence))) = (data(this), data(fence)) else { return E_INVALIDARG; };
    if value == u64::MAX || !Arc::ptr_eq(&queue.config, &fence.config) { return E_INVALIDARG; }
    submit_queue(&queue, QueueOp::Wait(fence, value))
}

#[repr(C)]
struct ListDescX {
    kind: u32,
    flags: u32,
    nodes: u32,
    barrier_batch: u32,
}
extern "system" fn create_list_x(
    this: This, d: *const ListDescX, alloc: This, pso: This, iid: *const Iid, out: *mut This,
) -> HRESULT {
    if !out.is_null() { unsafe { *out = std::ptr::null(); } }
    let Some(d) = (unsafe { d.as_ref() }) else { return E_INVALIDARG; };
    if d.flags != 0 || d.nodes != 0 || d.barrier_batch != 0 {
        return E_INVALIDARG;
    }
    create_list(this, d.nodes, d.kind, alloc, pso, iid, out)
}
fn list(this: This) -> Arc<Mutex<ListState>> {
    if let Some(Data::List(l, _)) = data(this) {
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
    if l.closed || l.invalid {
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
        Some(Data::GraphicsPipeline(p)) if l.kind == 0 => l.graphics.pipeline = Some(p),
        _ => l.invalid = true,
    }
}
extern "system" fn set_heaps(this: This, n: u32, heaps: *const This) {
    let l = list(this);
    let mut l = l.lock().unwrap();
    if l.closed {
        trap("closed list");
    }
    if l.kind == 0 {
        if n == 0 || n > 2 || heaps.is_null() { l.invalid = true; return; }
        let hs = unsafe { std::slice::from_raw_parts(heaps, n as usize) };
        if hs.iter().any(|p| !matches!(data(*p), Some(Data::Heap(..)) | Some(Data::Sampler(_)))) {
            l.invalid = true; return;
        }
        l.graphics.heaps = hs.iter().map(|p| *p as usize).collect();
        l.graphics.texture = None;
        l.graphics.sampler = None;
        return;
    }
    if l.kind != 2 || n != 1 || heaps.is_null() {
        l.invalid = true;
        return;
    }
    let p = unsafe { *heaps };
    if let Some(Data::Heap(h, _)) = data(p) {
        l.heap = Some((p as usize, h));
        l.table = None;
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
    let binding = l.heap.as_ref().and_then(|(base, heap)| {
        let index = handle.checked_sub(*base as u64)?;
        (index < heap.lock().unwrap().entries.len() as u64).then_some((slot, index as u32))
    });
    if l.kind != 2 || !(1..=63).contains(&slot) || binding.is_none()
        || l.table.is_some_and(|(old_slot, _)| old_slot != slot)
        || l.cbv.as_ref().is_some_and(|cbv| cbv.slot == slot)
    {
        l.invalid = true;
    } else {
        l.table = binding;
    }
}
extern "system" fn graphics_table(this: This, slot: u32, handle: u64) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if !l.graphics.heaps.contains(&(handle as usize)) { l.invalid = true; return; }
    match data(handle as This) {
        Some(Data::Heap(_, h)) => {
            let texture = h.lock().unwrap().clone();
            if let Some(t) = texture { l.graphics.texture = Some((slot,h,t)); } else { l.invalid = true; }
        }
        Some(Data::Sampler(h)) => {
            let sampler = *h.lock().unwrap();
            if let Some(v) = sampler { l.graphics.sampler = Some((slot,h,v)); } else { l.invalid = true; }
        }
        _ => l.invalid = true,
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
extern "system" fn set_cbv(this: This, slot: u32, address: u64) {
    let l = list(this);
    let mut l = l.lock().unwrap();
    if l.closed {
        trap("closed list");
    }
    if l.kind != 2 || !(2..=63).contains(&slot) || address % 16 != 0
        || l.cbv.as_ref().is_some_and(|cbv| cbv.slot != slot)
        || l.table.is_some_and(|(table_slot, _)| table_slot == slot)
    {
        l.invalid = true;
        return;
    }
    let map = objects().lock().unwrap();
    let found = map.values().find_map(|entry| {
        let Data::Buffer(buffer) = &entry.data else { return None; };
        let offset = address.checked_sub(buffer.ptr() as u64)?;
        if offset < buffer.len() as u64 {
            Some(RootCbv { slot, buffer: buffer.clone(), offset: offset as usize })
        } else {
            None
        }
    });
    drop(map);
    if let Some(cbv) = found {
        l.cbv = Some(cbv);
    } else {
        l.invalid = true;
    }
}
// Retain both objects and reject descriptor changes before submission.
extern "system" fn dispatch(this: This, x: u32, y: u32, z: u32) {
    let l = list(this);
    let mut l = l.lock().unwrap();
    if l.closed {
        trap("closed list");
    }
    let Some(heap) = l.heap.as_ref().map(|h| h.1.clone()) else { l.invalid = true; return; };
    let (generation, descriptors) = {
        let h = heap.lock().unwrap();
        let Some(descriptors) = h.entries.iter().cloned().collect::<Option<Vec<_>>>() else {
            l.invalid = true; return;
        };
        (h.generation, descriptors)
    };
    let constants: Option<Vec<u32>> = l.constants.iter().copied().collect();
    let Some((table_slot, table_base)) = l.table else { l.invalid = true; return; };
    let legacy_cbv = table_slot == 1 && table_base == 0
        && matches!(descriptors.as_slice(), [BufferDescriptor::Uav(v)]
            if v.offset == 0 && v.length as usize == v.buffer.len());
    if l.kind != 2 || x == 0 || y == 0 || z == 0 || l.pipeline.is_none()
        || constants.is_none() || (l.cbv.is_some() && !legacy_cbv)
        || descriptors.iter().enumerate().any(|(i, d)|
            descriptors[..i].iter().any(|other| Arc::ptr_eq(&d.view().buffer, &other.view().buffer))
                || l.cbv.as_ref().is_some_and(|cbv| Arc::ptr_eq(&d.view().buffer, &cbv.buffer)))
    { l.invalid = true; return; }
    let job = Job {
        shader: l.pipeline.as_ref().unwrap().clone(),
        heap,
        generation,
        descriptors,
        table_slot,
        table_base,
        constants: constants.unwrap(),
        cbv: l.cbv.clone(),
        groups: [x, y, z],
    };
    l.jobs.push(job);
}
fn run(c: &Config, j: &Job) -> Result<(), String> {
    // Snapshot at execution, after earlier queued jobs have updated resources.
    for descriptor in &j.descriptors {
        let state = *descriptor.view().buffer.state.lock().unwrap();
        match descriptor {
            BufferDescriptor::Srv(_) if !matches!(state, 0 | 0x40) =>
                return Err("SRV requires COMMON or NON_PIXEL_SHADER_RESOURCE state".into()),
            BufferDescriptor::Uav(_) if !matches!(state, 0 | 8) =>
                return Err("UAV requires COMMON or UNORDERED_ACCESS state".into()),
            _ => {}
        }
    }
    if let Some(binding) = &j.cbv {
        if !matches!(*binding.buffer.state.lock().unwrap(), 0 | 1) {
            return Err("root CBV requires COMMON or VERTEX_AND_CONSTANT_BUFFER state".into());
        }
    }
    let cbv = j.cbv.as_ref().map(|binding| {
        let len = (binding.buffer.len() - binding.offset).min(65536);
        let bytes = unsafe {
            std::slice::from_raw_parts(binding.buffer.ptr().add(binding.offset), len)
        }.to_vec();
        (binding.slot, bytes)
    });
    let legacy = if j.table_slot == 1 && j.table_base == 0 {
        match j.descriptors.as_slice() {
            [BufferDescriptor::Uav(uav)] if uav.offset == 0 && uav.length as usize == uav.buffer.len() =>
                Some((None, uav)),
            [BufferDescriptor::Srv(srv), BufferDescriptor::Uav(uav)] if cbv.is_none() =>
                Some((Some(srv), uav)),
            _ => None,
        }
    } else { None };
    let response_len = j.descriptors.iter().try_fold(0usize, |sum, descriptor| {
        if let BufferDescriptor::Uav(view) = descriptor { sum.checked_add(view.buffer.len()) } else { Some(sum) }
    }).ok_or("compute response too large")?;
    let payload_len = j.descriptors.iter().try_fold(
        32usize.checked_add(j.shader.len()).and_then(|n| n.checked_add(j.constants.len() * 4))
            .ok_or("compute payload too large")?,
        |sum, descriptor| sum.checked_add(16)?.checked_add(descriptor.view().buffer.len()),
    ).ok_or("compute payload too large")?;
    if payload_len > MAX_COMPUTE_PAYLOAD || response_len > MAX_COMPUTE_PAYLOAD {
        return Err("compute payload or response too large".into());
    }
    let mut s = TcpStream::connect_timeout(&c.addr, Duration::from_secs(10))
        .map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(60))).map_err(|e| e.to_string())?;
    s.set_write_timeout(Some(Duration::from_secs(60))).map_err(|e| e.to_string())?;
    let header = if let Some((srv, uav)) = legacy {
        [MAGIC, if srv.is_some() { 7 } else if cbv.is_some() { 6 } else { 1 },
         j.shader.len() as u32, uav.buffer.len() as u32, j.constants.len() as u32,
         j.groups[0], j.groups[1], j.groups[2]]
    } else {
        [MAGIC, 8, payload_len as u32, 0, 0, 0, 0, 0]
    };
    for value in header {
        s.write_all(&value.to_le_bytes()).map_err(|e| e.to_string())?;
    }
    s.write_all(&c.token).map_err(|e| e.to_string())?;
    if let Some((srv, uav)) = legacy {
        if let Some(srv) = srv {
            for value in [srv.buffer.len() as u32, srv.offset, srv.length, uav.offset, uav.length] {
                s.write_all(&value.to_le_bytes()).map_err(|e| e.to_string())?;
            }
        }
        if let Some((slot, bytes)) = &cbv {
            s.write_all(&slot.to_le_bytes())
                .and_then(|_| s.write_all(&(bytes.len() as u32).to_le_bytes()))
                .map_err(|e| e.to_string())?;
        }
        let input = unsafe { std::slice::from_raw_parts(uav.buffer.ptr(), uav.buffer.len()) };
        s.write_all(&j.shader).and_then(|_| s.write_all(input)).map_err(|e| e.to_string())?;
        for value in &j.constants {
            s.write_all(&value.to_le_bytes()).map_err(|e| e.to_string())?;
        }
        if let Some((_, bytes)) = &cbv {
            s.write_all(bytes).map_err(|e| e.to_string())?;
        }
        if let Some(srv) = srv {
            let bytes = unsafe { std::slice::from_raw_parts(srv.buffer.ptr(), srv.buffer.len()) };
            s.write_all(bytes).map_err(|e| e.to_string())?;
        }
    } else {
        for value in [j.shader.len() as u32, j.constants.len() as u32, j.table_slot, j.table_base,
            j.descriptors.len() as u32, j.groups[0], j.groups[1], j.groups[2]] {
            s.write_all(&value.to_le_bytes()).map_err(|e| e.to_string())?;
        }
        s.write_all(&j.shader).map_err(|e| e.to_string())?;
        for value in &j.constants {
            s.write_all(&value.to_le_bytes()).map_err(|e| e.to_string())?;
        }
        for descriptor in &j.descriptors {
            let view = descriptor.view();
            let kind = match descriptor { BufferDescriptor::Srv(_) => 1u32, BufferDescriptor::Uav(_) => 2 };
            for value in [kind, view.offset, view.length, view.buffer.len() as u32] {
                s.write_all(&value.to_le_bytes()).map_err(|e| e.to_string())?;
            }
            let bytes = unsafe { std::slice::from_raw_parts(view.buffer.ptr(), view.buffer.len()) };
            s.write_all(bytes).map_err(|e| e.to_string())?;
        }
    }
    let mut header = [0u8; 16];
    s.read_exact(&mut header).map_err(|e| e.to_string())?;
    let word = |n: usize| u32::from_le_bytes(header[n..n + 4].try_into().unwrap());
    let (magic, status, len, msg) = (word(0), word(4), word(8) as usize, word(12) as usize);
    if magic != MAGIC || len > MAX_COMPUTE_PAYLOAD || msg > 4096
        || (status == 0 && (len != response_len || msg != 0))
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
    let mut offset = 0;
    for descriptor in &j.descriptors {
        if let BufferDescriptor::Uav(view) = descriptor {
            let size = view.buffer.len();
            unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr().add(offset), view.buffer.ptr(), size); }
            offset += size;
        }
    }
    tracing::info!("Wine Metal proxy connected");
    Ok(())
}
fn perform_queue_op(queue: &ProxyQueue, op: QueueOp) -> Result<(), String> {
    match op {
        QueueOp::Wait(fence, value) => fence.wait(value),
        QueueOp::Signal(fence, value) => fence.signal(value),
        QueueOp::Compute(jobs) => {
            let _submission = SUBMISSION.lock().unwrap_or_else(|e| e.into_inner());
            for job in &jobs {
                let descriptor = job.heap.lock().unwrap_or_else(|e| e.into_inner());
                if descriptor.generation != job.generation {
                    return Err("buffer descriptor changed after dispatch".into());
                }
                run(&queue.config, job)?;
            }
        }
        QueueOp::Graphics(jobs) => {
            if !jobs.is_empty() {
                let _submission = SUBMISSION.lock().unwrap_or_else(|e| e.into_inner());
                run_graphics(&queue.config, &jobs)?;
            }
        }
    }
    Ok(())
}

// Idle queues keep the existing synchronous submission contract. An
// unsatisfied queue wait starts a per-queue worker; later operations are
// retained in FIFO order without blocking the caller or the global GPU lock.
fn submit_queue(queue: &Arc<ProxyQueue>, op: QueueOp) -> HRESULT {
    let mut state = queue.state.lock().unwrap_or_else(|e| e.into_inner());
    if state.active {
        state.pending.push_back(op);
        return S_OK;
    }
    if let QueueOp::Wait(fence, value) = &op {
        if fence.value() >= *value { return S_OK; }
        state.pending.push_back(op);
        state.active = true;
        let queue = queue.clone();
        if std::thread::Builder::new().name("victory-wine-queue".into()).spawn(move || {
            loop {
                let next = {
                    let mut state = queue.state.lock().unwrap_or_else(|e| e.into_inner());
                    match state.pending.pop_front() {
                        Some(op) => op,
                        None => { state.active = false; return; }
                    }
                };
                if let Err(error) = perform_queue_op(&queue, next) { trap(&error); }
            }
        }).is_err() {
            state.active = false;
            state.pending.clear();
            return FAIL;
        }
        return S_OK;
    }
    if let Err(error) = perform_queue_op(queue, op) { trap(&error); }
    S_OK
}

extern "system" fn execute(this: This, count: u32, lists: *const This) {
    let Some(Data::Queue(queue)) = data(this) else { trap("invalid queue"); };
    if count == 0 || lists.is_null() { trap("invalid command lists"); }
    let mut graphics_jobs = Vec::new();
    let mut compute_jobs = Vec::new();
    for &p in unsafe { std::slice::from_raw_parts(lists, count as usize) } {
        let l = list(p);
        let l = l.lock().unwrap();
        if !l.closed || l.invalid || l.kind != queue.kind { trap("invalid or open command list"); }
        if l.kind == 0 {
            graphics_jobs.extend(l.graphics.jobs.iter().cloned());
        } else {
            compute_jobs.extend(l.jobs.iter().cloned());
        }
    }
    let op = if queue.kind == 0 { QueueOp::Graphics(graphics_jobs) } else { QueueOp::Compute(compute_jobs) };
    if submit_queue(&queue, op) != S_OK { trap("could not schedule queue submission"); }
}

extern "system" fn execute_x(this: This, count: u32, lists: *const This, _token: u64, flags: u32) {
    if flags != 0 { trap("unsupported command submission flags"); }
    // Frame tokens are pacing metadata; submission order is preserved.
    execute(this, count, lists);
}

extern "system" fn present(this: This, planes: u32, plane: *const crate::queue::Plane, parameters: *const u8) -> HRESULT {
    let Some(Data::Queue(queue)) = data(this) else { return E_INVALIDARG; };
    if queue.kind != 0 { return E_INVALIDARG; }
    if planes != 1 || plane.is_null() || !parameters.is_null() { return E_INVALIDARG; }
    let plane = unsafe { &*plane };
    if plane.count != 1 || plane.resources.is_null() { return E_INVALIDARG; }
    let state = queue.state.lock().unwrap_or_else(|e| e.into_inner());
    if state.active { return crate::object::E_NOTIMPL; }
    let _submission = SUBMISSION.lock().unwrap_or_else(|e| e.into_inner());
    let Some(Data::Texture(texture)) = data(unsafe { *plane.resources }) else { return E_INVALIDARG; };
    if texture.format != 28 { return E_INVALIDARG; }
    let bytes = texture.bytes.lock().unwrap_or_else(|e| e.into_inner());
    match crate::wine_present::send(queue.config.addr, queue.config.token, texture.width, texture.height, texture.format, texture.width * 4, &bytes, false) {
        Ok(_) => S_OK,
        Err(e) => { tracing::error!(%e, "Wine presentation failed"); FAIL }
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
    if device(this).is_none() || d.size == 0 || d.size > 4096 || d.stream.is_null() { return E_INVALIDARG; }
    let b = unsafe { std::slice::from_raw_parts(d.stream, d.size) };
    let mut at = 0usize;
    let mut seen = 0u32;
    let (mut vp,mut vl,mut pp,mut pl,mut ep) = (0,0,0,0,0);
    let mut input_count = 0;
    let mut depth = None;
    let mut stencil_state = None;
    let mut front_counter_clockwise = false;
    let mut depth_format = 0;
    let mut blend = Blend::new(false, 15, [0; 6]).unwrap();
    while at < b.len() {
        if b.len() - at < 8 { return E_INVALIDARG; }
        let kind = u32::from_ne_bytes(b[at..at+4].try_into().unwrap());
        let size = match kind { 0 => 16, 1 | 2 | 12 => 24, 8 => 336, 9 | 13 | 14 | 16 => 8, 10 => 48, 11 => 56, 15 => 40, 17 => 16, _ => return E_INVALIDARG };
        if at + size > b.len() || seen & (1 << kind) != 0 { return E_INVALIDARG; }
        seen |= 1 << kind;
        let word = |offset: usize| u32::from_ne_bytes(b[at+offset..at+offset+4].try_into().unwrap());
        match kind {
            0 => if stream_ptr(b,at+8) != 0 { return E_INVALIDARG; },
            1 => { vp = stream_ptr(b,at+8); vl = stream_ptr(b,at+16); }
            2 => { pp = stream_ptr(b,at+8); pl = stream_ptr(b,at+16); }
            8 => {
                let enabled = word(12);
                let mask = b[at+48];
                if word(4) != 0 || word(8) != 0 || enabled > 1 || word(16) != 0 || mask > 15 {
                    return E_INVALIDARG;
                }
                let factors = [word(20),word(24),word(28),word(32),word(36),word(40)];
                let Some(state) = Blend::new(enabled == 1, mask, factors) else { return E_INVALIDARG; };
                blend = state;
            }
            9 => if word(4) != u32::MAX { return E_INVALIDARG; },
            12 => { ep = stream_ptr(b,at+8); input_count = word(16); if !(1..=16).contains(&input_count) { return E_INVALIDARG; } }
            10 => {
                if word(4) != 3 || word(8) != 1 || (16..48).step_by(4).any(|i| word(i) != if i == 28 { 1 } else { 0 }) { return E_INVALIDARG; }
                front_counter_clockwise = word(12) != 0;
            }
            11 => {
                let (enabled, write, compare, stencil) = (word(4),word(8),word(12),word(16));
                if enabled > 1 || stencil > 1 || (enabled == 1 && (write > 1 || !(1..=8).contains(&compare))) { return E_INVALIDARG; }
                if enabled == 1 || stencil == 1 {
                    depth = Some(PipelineDepth { write: enabled == 1 && write == 1, compare: if enabled == 1 {compare} else {8}, stencil: None });
                }
                let s = Stencil { enabled: stencil == 1, read_mask: b[at+20], write_mask: b[at+21],
                    front: [word(24),word(28),word(32),word(36)],
                    back: [word(40),word(44),word(48),word(52)] };
                if s.enabled && !s.valid() { return E_INVALIDARG; }
                stencil_state = Some(s);
            }
            13 => if word(4) != 0 { return E_INVALIDARG; },
            14 => if word(4) != 3 { return E_INVALIDARG; },
            15 => for offset in (4..40).step_by(4) {
                let expected = match offset { 4 => 28, 36 => 1, _ => 0 };
                if word(offset) != expected { return E_INVALIDARG; }
            },
            16 => { depth_format = word(4); if !matches!(depth_format, 0 | 20 | 40) { return E_INVALIDARG; } }
            17 => if word(4) != 1 || word(8) != 0 { return E_INVALIDARG; },
            _ => unreachable!(),
        }
        at += size;
    }
    let required = 1<<1 | 1<<2 | 1<<10 | 1<<11 | 1<<12 | 1<<14 | 1<<15;
    if seen & required != required || (depth.is_some() && !matches!(depth_format, 20 | 40))
        || (depth_format == 0 && depth.is_some()) || (depth_format == 40 && stencil_state.as_ref().is_some_and(|s| s.enabled))
        || (depth.is_none() && depth_format != 0) { return E_INVALIDARG; }
    if front_counter_clockwise && stencil_state.as_ref().is_some_and(|s| s.enabled) { return E_INVALIDARG; }
    if depth_format == 20 {
        let Some(d) = depth.as_mut() else { return E_INVALIDARG; };
        d.stencil = stencil_state;
    }
    if vp == 0 || pp == 0 || ep == 0 || vl == 0 || pl == 0 || vl > MAX_SHADER || pl > MAX_SHADER { return E_INVALIDARG; }
    let mut inputs = Vec::with_capacity(input_count as usize);
    let mut next = [0u32;16];
    for i in 0..input_count as usize {
        let e = unsafe { &*((ep as *const InputElement).add(i)) };
        if e.name.is_null() || e.slot >= 16 || e.class > 1 { return E_INVALIDARG; }
        let Ok(name) = unsafe { std::ffi::CStr::from_ptr(e.name.cast()) }.to_str() else { return E_INVALIDARG; };
        let Some(size) = wine_transport::vertex_format_size(e.format) else { return E_INVALIDARG; };
        let align = if matches!(e.format, 34 | 35) { 2 } else { 4 };
        let offset = if e.offset == u32::MAX {
            let Some(v)=next[e.slot as usize].checked_add(align-1) else { return E_INVALIDARG; };
            v & !(align-1)
        } else { e.offset };
        let Some(end)=offset.checked_add(size) else { return E_INVALIDARG; };
        next[e.slot as usize] = end;
        inputs.push(VertexInput { semantic: name.to_owned(), semantic_index:e.index, format:e.format,
            slot:e.slot, offset, per_instance:e.class==1, step_rate:e.step });
    }
    if wine_transport::validate_vertex_inputs(&inputs).is_err() { return E_INVALIDARG; }
    let legacy = inputs[0].semantic.eq_ignore_ascii_case("POSITION") && inputs[0].semantic_index==0
        && inputs[0].format==2 && inputs[0].slot==0 && inputs[0].offset==0 && !inputs[0].per_instance
        && (inputs.len()==1 || (inputs.len()==2 && inputs[1].semantic.eq_ignore_ascii_case("TEXCOORD")
            && inputs[1].semantic_index==0 && inputs[1].format==16 && inputs[1].slot==0
            && inputs[1].offset==16 && !inputs[1].per_instance));
    let p = GraphicsPipeline { id: resource_id(), vs: unsafe { std::slice::from_raw_parts(vp as *const u8,vl) }.to_vec(), ps: unsafe { std::slice::from_raw_parts(pp as *const u8,pl) }.to_vec(), stride: if inputs.len()==2 { 24 } else { 16 }, inputs, legacy, depth, blend };
    new(Data::GraphicsPipeline(Arc::new(p)), &PIPELINE_TABLE, iid, PIPELINE, out)
}
extern "system" fn gpu_address(this: This) -> u64 {
    match data(this) { Some(Data::Buffer(b)) => b.ptr() as u64, _ => trap("invalid GPU address") }
}
extern "system" fn create_rtv(this: This, resource: This, desc: *const c_void, handle: u64) {
    let (Some(Data::Texture(t)), Some(Data::Rtv(h))) = (data(resource), data(handle as This)) else { trap("invalid RTV"); };
    if device(this).is_none() || !desc.is_null() || t.format != 28 || t.tiled.is_some() { trap("unsupported RTV"); }
    let mut d = h.lock().unwrap();
    d.0 = Some(t);
    d.1 = d.1.checked_add(1).unwrap_or_else(|| trap("RTV revision overflow"));
}
extern "system" fn create_dsv(this: This, resource: This, desc: *const c_void, handle: u64) {
    let (Some(Data::Texture(t)), Some(Data::Dsv(h))) = (data(resource), data(handle as This)) else { trap("invalid DSV"); };
    if device(this).is_none() || !desc.is_null() || !matches!(t.format, 20 | 40) { trap("unsupported DSV"); }
    let mut d = h.lock().unwrap();
    d.0 = Some(t);
    d.1 = d.1.checked_add(1).unwrap_or_else(|| trap("DSV revision overflow"));
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
fn dsv(handle: u64) -> Option<Rtv> {
    let Some(Data::Dsv(h)) = data(handle as This) else { return None; };
    let (t, revision) = { let d = h.lock().unwrap(); (d.0.clone()?, d.1) };
    Some((h,t,revision))
}
extern "system" fn clear_dsv(this: This, handle: u64, flags: u32, depth: f32, stencil: u8, n: u32, rects: *const c_void) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    let Some(desc) = dsv(handle) else { l.invalid = true; return; };
    if n != 0 || !rects.is_null() || flags == 0 || flags & !3 != 0
        || (flags & 1 != 0 && (!depth.is_finite() || !(0.0..=1.0).contains(&depth)))
        || (flags & 2 != 0 && desc.1.format != 20) { l.invalid = true; return; }
    let resource = GraphicsResource::Texture(desc.1.clone());
    let command = if desc.1.format == 20 { Command::ClearStencil { target: resource.id(), flags, depth, stencil } }
        else { Command::ClearDepth { target: resource.id(), depth } };
    l.graphics.jobs.push(GraphicsJob { command, resources: vec![resource], descriptor: Some(desc),
        depth_descriptor: None, pipeline: None, texture: None, sampler: None });
}
extern "system" fn clear_rtv(this: This, handle: u64, color: *const f32, n: u32, rects: *const c_void) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    let Some(desc) = rtv(handle) else { l.invalid = true; return; };
    if color.is_null() || n != 0 || !rects.is_null() { l.invalid = true; return; }
    let c = unsafe { *(color as *const [f32;4]) };
    if !c.iter().all(|v| v.is_finite()) { l.invalid = true; return; }
    let resource = GraphicsResource::Texture(desc.1.clone());
    l.graphics.jobs.push(GraphicsJob { command: Command::Clear { target: resource.id(), color: c }, resources: vec![resource], descriptor: Some(desc), depth_descriptor: None, pipeline: None, texture: None, sampler: None });
}
extern "system" fn targets(this: This, n: u32, handles: *const u64, single: i32, depth: *const u64) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if n != 1 || handles.is_null() || !matches!(single, 0 | 1) { l.invalid = true; return; }
    l.graphics.target = rtv(unsafe { *handles });
    l.graphics.depth = if depth.is_null() { None } else { dsv(unsafe { *depth }) };
    if l.graphics.target.is_none() || (!depth.is_null() && l.graphics.depth.is_none()) { l.invalid = true; }
}
extern "system" fn set_stencil_ref(this: This, value: u32) {
    let l = graphics_list(this); l.lock().unwrap().graphics.stencil_ref = value & 255;
}
extern "system" fn topology(this: This, value: u32) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if value != 4 { l.invalid = true; } else { l.graphics.topology = true; }
}
extern "system" fn viewport(this: This, n: u32, p: *const [f32; 6]) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if n != 1 || p.is_null() { l.invalid = true; } else { l.graphics.viewport = Some(unsafe { *p }); }
}
extern "system" fn scissor(this: This, n: u32, p: *const [i32; 4]) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if n != 1 || p.is_null() { l.invalid = true; } else { l.graphics.scissor = Some(unsafe { *p }); }
}
#[repr(C)]
struct VertexView { address: u64, size: u32, stride: u32 }
extern "system" fn vertices(this: This, slot: u32, n: u32, p: *const VertexView) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if n == 0 || slot.checked_add(n).is_none_or(|end|end>16) || p.is_null() { l.invalid = true; return; }
    let views = unsafe { std::slice::from_raw_parts(p,n as usize) };
    let map = objects().lock().unwrap();
    let mut resolved = Vec::with_capacity(n as usize);
    for v in views {
        if v.stride==0 || v.stride>2048 || v.size==0 { l.invalid=true; return; }
        let found = map.values().find_map(|e| if let Data::Buffer(b) = &e.data {
            let offset = v.address.checked_sub(b.ptr() as u64)?;
            if offset.checked_add(v.size as u64)? <= b.len() as u64 { Some((b.clone(),offset as usize,v.size,v.stride)) } else { None }
        } else { None });
        let Some(found)=found else { l.invalid=true; return; };
        resolved.push(found);
    }
    drop(map);
    for (i,v) in resolved.into_iter().enumerate() { l.graphics.vertices[slot as usize+i]=Some(v); }
}
extern "system" fn graphics_constant(this: This, slot: u32, value: u32, offset: u32) {
    graphics_constants(this, slot, 1, &value, offset);
}
#[repr(C)]
struct IndexView { address: u64, size: u32, format: u32 }
extern "system" fn indices(this: This, p: *const IndexView) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if p.is_null() { l.graphics.index = None; return; }
    let v = unsafe { &*p };
    if !matches!(v.format, 42 | 57) || v.size == 0 { l.invalid = true; return; }
    let map = objects().lock().unwrap();
    let found = map.values().find_map(|e| if let Data::Buffer(b) = &e.data {
        let offset = v.address.checked_sub(b.ptr() as u64)?;
        if offset.checked_add(v.size as u64)? <= b.len() as u64 { Some((b.clone(), offset as usize, v.size, v.format)) } else { None }
    } else { None });
    drop(map);
    if found.is_none() { l.invalid = true; } else { l.graphics.index = found; }
}
extern "system" fn graphics_constants(this: This, slot: u32, n: u32, values: *const u32, offset: u32) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    let end = (offset as usize).checked_add(n as usize);
    if values.is_null() || end.is_none_or(|v| v > 64) || l.graphics.constant_slot.is_some_and(|s| s != slot) { l.invalid = true; return; }
    l.graphics.constant_slot = Some(slot);
    let end = end.unwrap();
    let c = &mut l.graphics.constants;
    if c.len() < end { c.resize(end, None); }
    for (dst, value) in c[offset as usize..end].iter_mut().zip(unsafe { std::slice::from_raw_parts(values, n as usize) }) { *dst = Some(*value); }
}
extern "system" fn draw(this: This, count: u32, instances: u32, first: u32, instance: u32) {
    record_draw(this, count, instances, first, instance, None);
}
extern "system" fn draw_indexed(this: This, count: u32, instances: u32, first: u32, base: i32, instance: u32) {
    record_draw(this, count, instances, first, instance, Some(base));
}
fn record_draw(this: This, count: u32, instances: u32, first: u32, instance: u32, base: Option<i32>) {
    let l = graphics_list(this); let mut l = l.lock().unwrap(); let g = &mut l.graphics;
    let (Some(desc), Some(pipeline), Some(viewport), Some(scissor), Some(constants)) =
        (g.target.clone(), g.pipeline.clone(), g.viewport, g.scissor, g.constants.iter().copied().collect::<Option<Vec<_>>>())
        else { l.invalid = true; return; };
    let index = if let Some(base_vertex) = base {
        let Some((b, offset, size, format)) = g.index.clone() else { l.invalid = true; return; };
        let element = if format == 57 { 2 } else { 4 };
        if first.checked_add(count).and_then(|n| n.checked_mul(element)).is_none_or(|n| n > size) { l.invalid = true; return; }
        Some((b.clone(), Index { buffer: b.id, offset: offset as u64, size, format, first, base_vertex }))
    } else { None };
    if !g.topology || count == 0 || instances == 0
        || !viewport.iter().all(|v| v.is_finite()) || viewport[2] <= 0.0 || viewport[3] <= 0.0
        || scissor[0] < 0 || scissor[1] < 0 || scissor[2] <= scissor[0] || scissor[3] <= scissor[1] {
        l.invalid = true; return;
    }
    let target = desc.1.clone();
    let mut resources = vec![GraphicsResource::Texture(target.clone())];
    let mut views = Vec::new();
    for a in &pipeline.inputs {
        let Some((buffer,offset,size,stride)) = g.vertices[a.slot as usize].as_ref() else { l.invalid=true; return; };
        let max = if a.per_instance {
            instance.checked_add((instances-1)/a.step_rate)
        } else if index.is_none() { first.checked_add(count-1) } else { None };
        if let Some(max)=max {
            let end=u64::from(max).checked_mul(u64::from(*stride))
                .and_then(|v|v.checked_add(u64::from(a.offset)))
                .and_then(|v|v.checked_add(u64::from(wine_transport::vertex_format_size(a.format).unwrap())));
            if end.is_none_or(|v|v>u64::from(*size)) { l.invalid=true; return; }
        } else if a.per_instance || index.is_none() { l.invalid=true; return; }
        if !views.iter().any(|v: &wine_transport::VertexView|v.slot==a.slot) {
            views.push(wine_transport::VertexView {slot:a.slot,buffer:buffer.id,offset:*offset as u64,size:*size,stride:*stride});
            resources.push(GraphicsResource::Buffer(buffer.clone()));
        }
    }
    let legacy=pipeline.legacy && instances==1 && instance==0 && views.len()==1
        && views[0].slot==0 && views[0].stride==pipeline.stride;
    let first_view=views.first().unwrap().clone();
    if let Some((b,_)) = &index { resources.push(GraphicsResource::Buffer(b.clone())); }
    if let Some((_,_,view)) = &g.texture { resources.push(GraphicsResource::Texture(view.texture.clone())); }
    if g.texture.is_some() != g.sampler.is_some() { l.invalid = true; return; }
    if pipeline.depth.as_ref().is_some_and(|d| d.stencil.is_some()) {
        g.jobs.push(GraphicsJob { command: Command::SetStencilRef(g.stencil_ref), resources: vec![],
            descriptor: None, depth_descriptor: None, pipeline: None, texture: None, sampler: None });
    }
    if pipeline.depth.is_some() != g.depth.is_some() { l.invalid = true; return; }
    if let Some((_,t,_)) = &g.depth { resources.push(GraphicsResource::Texture(t.clone())); }
    let command = Command::Draw(Draw { target: target.id, pipeline: pipeline.id,
        vertex_buffer: first_view.buffer, vertex_offset: first_view.offset, vertex_size: first_view.size, vertex_stride: first_view.stride,
        vertex_count: count, instance_count: instances, first_vertex: if index.is_some() { 0 } else { first }, first_instance: instance,
        viewport, scissor, topology: 4, constants, texture: g.texture.as_ref().map(|v| v.2.texture.id),
        first_mip: g.texture.as_ref().map_or(0, |v| v.2.first_mip),
        mip_count: g.texture.as_ref().map_or(0, |v| v.2.mip_count),
        index: index.map(|v| v.1), depth: g.depth.as_ref().map(|v| v.1.id), linear_filter: g.sampler.as_ref().is_some_and(|v| v.2),
        constant_slot: g.constant_slot, texture_slot: g.texture.as_ref().map(|v| v.0),
        sampler_slot: g.sampler.as_ref().map(|v| v.0), vertices: if legacy {Vec::new()} else {views} });
    g.jobs.push(GraphicsJob { command, resources, descriptor: Some(desc), depth_descriptor: g.depth.clone(), pipeline: Some(pipeline),
        texture: g.texture.as_ref().map(|v| (v.1.clone(),v.2.clone())),
        sampler: g.sampler.as_ref().map(|v| (v.1.clone(),v.2)) });
}
extern "system" fn copy_buffer(this: This, dst: This, dst_offset: u64, src: This, src_offset: u64, size: u64) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    let (Some(Data::Buffer(d)),Some(Data::Buffer(s))) = (data(dst),data(src)) else { l.invalid = true; return; };
    if size == 0 || Arc::ptr_eq(&d,&s) || dst_offset.checked_add(size).is_none_or(|v| v > d.len() as u64)
        || src_offset.checked_add(size).is_none_or(|v| v > s.len() as u64) { l.invalid = true; return; }
    let command = Command::CopyBuffer { src: s.id, dst: d.id, src_offset, dst_offset, size };
    l.graphics.jobs.push(GraphicsJob { command, resources: vec![GraphicsResource::Buffer(s),GraphicsResource::Buffer(d)],
        descriptor: None, depth_descriptor: None, pipeline: None, texture: None, sampler: None });
}
#[repr(C)]
struct Transition { kind: u32, flags: u32, resource: This, subresource: u32, before: u32, after: u32, padding: u32 }
extern "system" fn barrier(this: This, n: u32, p: *const Transition) -> u32 {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if n == 0 || n > 4096 || p.is_null() { l.invalid = true; return 0; }
    for b in unsafe { std::slice::from_raw_parts(p, n as usize) } {
        let resource = match data(b.resource) { Some(Data::Texture(t)) => GraphicsResource::Texture(t), Some(Data::Buffer(b)) => GraphicsResource::Buffer(b), _ => { l.invalid = true; return 0; } };
        if b.kind != 0 || b.flags != 0 || b.subresource != u32::MAX { l.invalid = true; return 0; }
        l.graphics.jobs.push(GraphicsJob { command: Command::Barrier { id: resource.id(), before: b.before, after: b.after }, resources: vec![resource], descriptor: None, depth_descriptor: None, pipeline: None, texture: None, sampler: None });
    }
    0
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Footprint { offset: u64, format: u32, width: u32, height: u32, depth: u32, pitch: u32 }
#[repr(C)]
union CopyLocationData { footprint: Footprint, subresource: u32 }
#[repr(C)]
struct CopyLocation { resource: This, kind: u32, data: CopyLocationData }
extern "system" fn copy_texture(this: This, dst: *const CopyLocation, x: u32, y: u32, z: u32, src: *const CopyLocation, region: *const c_void) {
    let l = graphics_list(this); let mut l = l.lock().unwrap();
    if dst.is_null() || src.is_null() || x != 0 || y != 0 || z != 0 || !region.is_null() { l.invalid = true; return; }
    let d = unsafe { &*dst }; let s = unsafe { &*src };
    let (buffer, texture, footprint, subresource, upload) = match (data(d.resource), data(s.resource)) {
        (Some(Data::Buffer(b)), Some(Data::Texture(t))) => (b,t,d,s,false),
        (Some(Data::Texture(t)), Some(Data::Buffer(b))) => (b,t,s,d,true),
        _ => { l.invalid = true; return; }
    };
    if texture.tiled.is_some() || subresource.kind != 0 || footprint.kind != 1 { l.invalid = true; return; }
    // Read only the active member: a subresource index does not initialize
    // the rest of the D3D12_TEXTURE_COPY_LOCATION union.
    let mip = unsafe { subresource.data.subresource };
    let footprint = unsafe { footprint.data.footprint };
    if texture.format == 20 {
        let width = texture.width;
        let height = texture.height;
        let Some(row) = (width as u64).checked_mul(if mip == 0 {4} else {1}) else { l.invalid = true; return; };
        let end = (footprint.pitch as u64).checked_mul(u64::from(height - 1))
            .and_then(|n| footprint.offset.checked_add(n)).and_then(|n| n.checked_add(row));
        let valid_format = match mip { 0 => matches!(footprint.format, 39 | 41), 1 => matches!(footprint.format, 60 | 62), _ => false };
        if !valid_format || footprint.offset % 512 != 0 || footprint.width != width || footprint.height != height
            || footprint.depth != 1 || footprint.pitch % 256 != 0 || u64::from(footprint.pitch) < row
            || end.is_none_or(|n| n > buffer.len() as u64) { l.invalid = true; return; }
        let (src,dst) = if upload { (buffer.id,texture.id) } else { (texture.id,buffer.id) };
        let command = Command::CopyDepthStencil { src,dst,offset:footprint.offset,pitch:footprint.pitch,width,height,plane:mip,upload };
        l.graphics.jobs.push(GraphicsJob { command, resources: vec![GraphicsResource::Buffer(buffer),GraphicsResource::Texture(texture)],
            descriptor: None, depth_descriptor: None, pipeline: None, texture: None, sampler: None });
        return;
    }
    let Some((width, height, _, _)) = wine_transport::mip_layout(texture.format, texture.width, texture.height, texture.mips, mip) else { l.invalid = true; return; };
    let Some((row, rows)) = wine_transport::block_geometry(texture.format, width, height) else { l.invalid = true; return; };
    let end = (footprint.pitch as u64).checked_mul((rows - 1) as u64)
        .and_then(|n| footprint.offset.checked_add(n)).and_then(|n| n.checked_add(row as u64));
    let compressed = matches!(texture.format, 71 | 72 | 77 | 78);
    let footprint_width = if compressed { width.next_multiple_of(4) } else { width };
    let footprint_height = if compressed { height.next_multiple_of(4) } else { height };
    if footprint.offset % 512 != 0 || footprint.format != texture.format || footprint.width != footprint_width
        || footprint.height != footprint_height || footprint.depth != 1 || footprint.pitch % 256 != 0
        || (footprint.pitch as u64) < row as u64 || end.is_none_or(|n| n > buffer.len() as u64) { l.invalid = true; return; }
    let (src_id, dst_id) = if upload { (buffer.id, texture.id) } else { (texture.id, buffer.id) };
    let command = Command::CopyTexture(CopyTexture { src: src_id, dst: dst_id, offset: footprint.offset,
        pitch: footprint.pitch, width, height, format: texture.format, mip, buffer_to_texture: upload });
    l.graphics.jobs.push(GraphicsJob { command, resources: vec![GraphicsResource::Buffer(buffer), GraphicsResource::Texture(texture)], descriptor: None, depth_descriptor: None, pipeline: None, texture: None, sampler: None });
}
fn run_graphics(c: &Config, jobs: &[GraphicsJob]) -> Result<(), String> {
    let mut resources = HashMap::new();
    let mut pipelines = HashMap::new();
    let mut commands = Vec::with_capacity(jobs.len());
    for j in jobs {
        if let Some((h,t,revision)) = &j.descriptor {
            let d = h.lock().unwrap();
            if d.1 != *revision || !d.0.as_ref().is_some_and(|v| Arc::ptr_eq(v,t)) { return Err("RTV descriptor changed".into()); }
        }
        if let Some((h,t,revision)) = &j.depth_descriptor {
            let d = h.lock().unwrap();
            if d.1 != *revision || !d.0.as_ref().is_some_and(|v| Arc::ptr_eq(v,t)) { return Err("DSV descriptor changed".into()); }
        }
        if let Some((h,t)) = &j.texture {
            if !h.lock().unwrap().as_ref().is_some_and(|v| Arc::ptr_eq(&v.texture, &t.texture)
                && v.first_mip == t.first_mip && v.mip_count == t.mip_count) { return Err("SRV descriptor changed".into()); }
        }
        if let Some((h,v)) = &j.sampler {
            if *h.lock().unwrap() != Some(*v) { return Err("sampler descriptor changed".into()); }
        }
        if let Some(p) = &j.pipeline {
            let entry = pipelines.entry(p.id).or_insert_with(|| Pipeline { id: p.id, vertex_shader: p.vs.clone(), pixel_shader: p.ps.clone(), vertex_stride: p.stride,
                format: 28, depth: p.depth.clone(), blend: p.blend.clone(), inputs: Vec::new() });
            if matches!(&j.command,Command::Draw(d) if !d.vertices.is_empty()) { entry.inputs=p.inputs.clone(); }
        }
        for r in &j.resources { resources.entry(r.id()).or_insert_with(|| r.clone()); }
        commands.push(j.command.clone());
    }
    let batch = Batch { resources: resources.values().map(GraphicsResource::snapshot).collect::<Result<Vec<_>, _>>()?, pipelines: pipelines.into_values().collect(), commands };
    let payload = wine_transport::encode_batch(&batch)?;
    let payload_len = u32::try_from(payload.len()).map_err(|_| "batch too large")?;
    let mut s = TcpStream::connect_timeout(&c.addr, Duration::from_secs(10)).map_err(|e| e.to_string())?;
    s.set_read_timeout(Some(Duration::from_secs(60))).map_err(|e| e.to_string())?;
    s.set_write_timeout(Some(Duration::from_secs(60))).map_err(|e| e.to_string())?;
    for v in [MAGIC,5,payload_len,0,0,0,0,0] { s.write_all(&v.to_le_bytes()).map_err(|e| e.to_string())?; }
    s.write_all(&c.token).and_then(|_| s.write_all(&payload)).map_err(|e| e.to_string())?;
    let mut header = [0u8;16]; s.read_exact(&mut header).map_err(|e| e.to_string())?;
    let word = |n| u32::from_le_bytes(header[n..n+4].try_into().unwrap());
    let (magic,status,len,msg) = (word(0),word(4),word(8) as usize,word(12) as usize);
    if magic != MAGIC || len > 128 * 1024 * 1024 || msg > 4096 || (status == 0 && msg != 0) || (status != 0 && len != 0) { return Err("invalid proxy response".into()); }
    let mut bytes = vec![0;len]; let mut message = vec![0;msg];
    s.read_exact(&mut bytes).and_then(|_| s.read_exact(&mut message)).map_err(|e| e.to_string())?;
    if status != 0 { return Err(format!("proxy status {status}: {}", String::from_utf8_lossy(&message))); }
    let results = wine_transport::decode_results(&bytes)?;
    if results.len() != resources.len() { return Err("incomplete resource results".into()); }
    let mut seen = std::collections::HashSet::new();
    for r in &results {
        if !seen.insert(r.id) { return Err("duplicate resource result".into()); }
        match resources.get(&r.id) {
            Some(GraphicsResource::Buffer(b)) if matches!(r.kind, ResourceKind::Buffer) && r.bytes.len() == b.len() => {},
            Some(GraphicsResource::Texture(t)) if matches!(r.kind, ResourceKind::Texture { width, height, format, mips } if width == t.width && height == t.height && format == t.format && mips == t.mips)
                && wine_transport::texture_size(t.format, t.width, t.height, t.mips) == Some(r.bytes.len()) => {},
            _ => return Err("invalid resource result".into()),
        }
    }
    for r in results {
        match resources.get(&r.id).unwrap() {
            GraphicsResource::Buffer(b) => { unsafe { std::ptr::copy_nonoverlapping(r.bytes.as_ptr(), b.ptr(), r.bytes.len()); } *b.state.lock().unwrap() = r.state; },
            GraphicsResource::Texture(t) => { if t.tiled.is_none() { *t.bytes.lock().unwrap() = r.bytes; } *t.state.lock().unwrap() = r.state; },
        }
    }
    tracing::info!("Wine Metal proxy connected");
    Ok(())
}
