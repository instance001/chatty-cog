use std::ffi::{CString, c_char, c_int, c_void};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

use anyhow::{Context, Result, anyhow};
use libloading::{Library, Symbol};

use crate::app_paths::read_gguf_architecture;
use crate::llama_sys::*;

fn llama_runtime_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

#[derive(Default)]
struct RuntimeBackendState {
    backend_inited: bool,
    gpu_backends_loaded: bool,
    cpu_backends_loaded: bool,
    live_runtime_handles: usize,
}

fn llama_runtime_backend_state() -> MutexGuard<'static, RuntimeBackendState> {
    static STATE: OnceLock<Mutex<RuntimeBackendState>> = OnceLock::new();
    STATE
        .get_or_init(|| Mutex::new(RuntimeBackendState::default()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

pub struct Llama {
    runtime_dir: PathBuf,
    _ggml: Library,
    _lib: Library,

    ggml_backend_load:
        Symbol<'static, unsafe extern "C" fn(*const c_char) -> *mut ggml_backend_reg>,
    ggml_backend_load_all_from_path: Symbol<'static, unsafe extern "C" fn(*const c_char)>,

    llama_backend_init: Symbol<'static, unsafe extern "C" fn()>,
    llama_print_system_info: Symbol<'static, unsafe extern "C" fn() -> *const c_char>,

    llama_model_default_params: Symbol<'static, unsafe extern "C" fn() -> llama_model_params>,
    llama_context_default_params: Symbol<'static, unsafe extern "C" fn() -> llama_context_params>,
    llama_sampler_chain_default_params:
        Symbol<'static, unsafe extern "C" fn() -> llama_sampler_chain_params>,

    llama_model_load_from_file: Symbol<
        'static,
        unsafe extern "C" fn(*const c_char, llama_model_params) -> *mut llama_model,
    >,
    llama_model_free: Symbol<'static, unsafe extern "C" fn(*mut llama_model)>,

    llama_init_from_model: Symbol<
        'static,
        unsafe extern "C" fn(*mut llama_model, llama_context_params) -> *mut llama_context,
    >,
    llama_free: Symbol<'static, unsafe extern "C" fn(*mut llama_context)>,

    llama_model_get_vocab:
        Symbol<'static, unsafe extern "C" fn(*const llama_model) -> *const llama_vocab>,
    llama_model_chat_template:
        Symbol<'static, unsafe extern "C" fn(*const llama_model) -> *const c_char>,
    llama_model_n_embd: Symbol<'static, unsafe extern "C" fn(*const llama_model) -> c_int>,

    llama_chat_apply_template: Symbol<
        'static,
        unsafe extern "C" fn(
            *const c_char,
            *const llama_chat_message,
            usize,
            bool,
            *mut c_char,
            c_int,
        ) -> c_int,
    >,

    llama_tokenize: Symbol<
        'static,
        unsafe extern "C" fn(
            *const llama_vocab,
            *const c_char,
            c_int,
            *mut llama_token,
            c_int,
            bool,
            bool,
        ) -> c_int,
    >,
    llama_token_to_piece: Symbol<
        'static,
        unsafe extern "C" fn(
            *const llama_vocab,
            llama_token,
            *mut c_char,
            c_int,
            c_int,
            bool,
        ) -> c_int,
    >,
    llama_vocab_is_eog:
        Symbol<'static, unsafe extern "C" fn(*const llama_vocab, llama_token) -> bool>,

    llama_batch_init: Symbol<'static, unsafe extern "C" fn(c_int, c_int, c_int) -> llama_batch>,
    llama_batch_free: Symbol<'static, unsafe extern "C" fn(llama_batch)>,
    llama_decode: Symbol<'static, unsafe extern "C" fn(*mut llama_context, llama_batch) -> c_int>,
    llama_n_ctx: Symbol<'static, unsafe extern "C" fn(*const llama_context) -> u32>,
    llama_n_batch: Symbol<'static, unsafe extern "C" fn(*const llama_context) -> u32>,

    llama_set_embeddings: Symbol<'static, unsafe extern "C" fn(*mut llama_context, bool)>,
    llama_get_embeddings: Symbol<'static, unsafe extern "C" fn(*mut llama_context) -> *mut f32>,

    llama_sampler_chain_init:
        Symbol<'static, unsafe extern "C" fn(llama_sampler_chain_params) -> *mut llama_sampler>,
    llama_sampler_chain_add:
        Symbol<'static, unsafe extern "C" fn(*mut llama_sampler, *mut llama_sampler)>,
    llama_sampler_free: Symbol<'static, unsafe extern "C" fn(*mut llama_sampler)>,

    llama_sampler_init_top_k: Symbol<'static, unsafe extern "C" fn(c_int) -> *mut llama_sampler>,
    llama_sampler_init_top_p:
        Symbol<'static, unsafe extern "C" fn(f32, usize) -> *mut llama_sampler>,
    llama_sampler_init_temp: Symbol<'static, unsafe extern "C" fn(f32) -> *mut llama_sampler>,
    llama_sampler_init_dist: Symbol<'static, unsafe extern "C" fn(u32) -> *mut llama_sampler>,
    llama_sampler_init_greedy: Symbol<'static, unsafe extern "C" fn() -> *mut llama_sampler>,

    llama_sampler_sample: Symbol<
        'static,
        unsafe extern "C" fn(*mut llama_sampler, *mut llama_context, c_int) -> llama_token,
    >,
    llama_sampler_accept: Symbol<'static, unsafe extern "C" fn(*mut llama_sampler, llama_token)>,
}

#[derive(Debug, Clone, Copy)]
enum BackendMode {
    GpuAllowed { n_gpu_layers: i32 },
    CpuOnly,
}

#[derive(Debug, Clone, Copy)]
enum ContextProfile {
    Default,
    Safe,
}

unsafe extern "C" fn abort_callback(data: *mut c_void) -> bool {
    if data.is_null() {
        return false;
    }
    let cancel = unsafe { &*(data as *const AtomicBool) };
    cancel.load(Ordering::Relaxed)
}

impl Llama {
    pub fn load(runtime_dir: &Path) -> Result<Self> {
        // Ensure dependent DLLs resolve (ggml-vulkan.dll, libomp, etc.)
        prepend_to_path(runtime_dir)?;

        let runtime_dir = runtime_dir
            .canonicalize()
            .with_context(|| format!("canonicalize {}", runtime_dir.display()))?;

        let ggml_dll = runtime_dir.join("ggml.dll");
        let ggml = unsafe { Library::new(&ggml_dll) }
            .with_context(|| format!("load {}", ggml_dll.display()))?;

        let dll = runtime_dir.join("llama.dll");
        let lib =
            unsafe { Library::new(&dll) }.with_context(|| format!("load {}", dll.display()))?;

        unsafe fn sym<T>(lib: &Library, name: &[u8]) -> Result<Symbol<'static, T>> {
            let s: Symbol<T> = unsafe { lib.get(name) }.map_err(|e| anyhow!("{e}"))?;
            Ok(unsafe { std::mem::transmute::<Symbol<T>, Symbol<'static, T>>(s) })
        }

        let llama = Self {
            runtime_dir,
            ggml_backend_load: unsafe { sym(&ggml, b"ggml_backend_load\0") }?,
            ggml_backend_load_all_from_path: unsafe {
                sym(&ggml, b"ggml_backend_load_all_from_path\0")
            }?,
            llama_backend_init: unsafe { sym(&lib, b"llama_backend_init\0") }?,
            llama_print_system_info: unsafe { sym(&lib, b"llama_print_system_info\0") }?,
            llama_model_default_params: unsafe { sym(&lib, b"llama_model_default_params\0") }?,
            llama_context_default_params: unsafe { sym(&lib, b"llama_context_default_params\0") }?,
            llama_sampler_chain_default_params: unsafe {
                sym(&lib, b"llama_sampler_chain_default_params\0")
            }?,
            llama_model_load_from_file: unsafe { sym(&lib, b"llama_model_load_from_file\0") }?,
            llama_model_free: unsafe { sym(&lib, b"llama_model_free\0") }?,
            llama_init_from_model: unsafe { sym(&lib, b"llama_init_from_model\0") }?,
            llama_free: unsafe { sym(&lib, b"llama_free\0") }?,
            llama_model_get_vocab: unsafe { sym(&lib, b"llama_model_get_vocab\0") }?,
            llama_model_chat_template: unsafe { sym(&lib, b"llama_model_chat_template\0") }?,
            llama_model_n_embd: unsafe { sym(&lib, b"llama_model_n_embd\0") }?,
            llama_chat_apply_template: unsafe { sym(&lib, b"llama_chat_apply_template\0") }?,
            llama_tokenize: unsafe { sym(&lib, b"llama_tokenize\0") }?,
            llama_token_to_piece: unsafe { sym(&lib, b"llama_token_to_piece\0") }?,
            llama_vocab_is_eog: unsafe { sym(&lib, b"llama_vocab_is_eog\0") }?,
            llama_batch_init: unsafe { sym(&lib, b"llama_batch_init\0") }?,
            llama_batch_free: unsafe { sym(&lib, b"llama_batch_free\0") }?,
            llama_decode: unsafe { sym(&lib, b"llama_decode\0") }?,
            llama_n_ctx: unsafe { sym(&lib, b"llama_n_ctx\0") }?,
            llama_n_batch: unsafe { sym(&lib, b"llama_n_batch\0") }?,
            llama_set_embeddings: unsafe { sym(&lib, b"llama_set_embeddings\0") }?,
            llama_get_embeddings: unsafe { sym(&lib, b"llama_get_embeddings\0") }?,
            llama_sampler_chain_init: unsafe { sym(&lib, b"llama_sampler_chain_init\0") }?,
            llama_sampler_chain_add: unsafe { sym(&lib, b"llama_sampler_chain_add\0") }?,
            llama_sampler_free: unsafe { sym(&lib, b"llama_sampler_free\0") }?,
            llama_sampler_init_top_k: unsafe { sym(&lib, b"llama_sampler_init_top_k\0") }?,
            llama_sampler_init_top_p: unsafe { sym(&lib, b"llama_sampler_init_top_p\0") }?,
            llama_sampler_init_temp: unsafe { sym(&lib, b"llama_sampler_init_temp\0") }?,
            llama_sampler_init_dist: unsafe { sym(&lib, b"llama_sampler_init_dist\0") }?,
            llama_sampler_init_greedy: unsafe { sym(&lib, b"llama_sampler_init_greedy\0") }?,
            llama_sampler_sample: unsafe { sym(&lib, b"llama_sampler_sample\0") }?,
            llama_sampler_accept: unsafe { sym(&lib, b"llama_sampler_accept\0") }?,
            _ggml: ggml,
            _lib: lib,
        };

        {
            let mut state = llama_runtime_backend_state();
            state.live_runtime_handles = state.live_runtime_handles.saturating_add(1);
        }

        Ok(llama)
    }

    pub fn system_info(&self) -> String {
        let ptr = unsafe { (self.llama_print_system_info)() };
        if ptr.is_null() {
            return String::new();
        }
        unsafe { std::ffi::CStr::from_ptr(ptr) }
            .to_string_lossy()
            .trim()
            .to_string()
    }

    pub fn load_backends_gpu_allowed(&self) -> Result<()> {
        let cdir = CString::new(self.runtime_dir.to_string_lossy().as_bytes().to_vec())?;
        unsafe { (self.ggml_backend_load_all_from_path)(cdir.as_ptr()) };
        Ok(())
    }

    pub fn load_backends_cpu_only(&self) -> Result<()> {
        // Prefer arch-tuned DLL if present, but fall back to portable.
        let candidates = [
            "ggml-cpu-zen4.dll",
            "ggml-cpu-x64.dll",
            "ggml-cpu-sse42.dll",
        ];
        let mut picked = None;
        for c in candidates {
            let p = self.runtime_dir.join(c);
            if p.is_file() {
                picked = Some(p);
                break;
            }
        }
        let picked = picked.ok_or_else(|| {
            anyhow!(
                "no ggml cpu backend dll found in {}",
                self.runtime_dir.display()
            )
        })?;
        let cpath = CString::new(picked.to_string_lossy().as_bytes().to_vec())?;
        let reg = unsafe { (self.ggml_backend_load)(cpath.as_ptr()) };
        if reg.is_null() {
            return Err(anyhow!("failed to load CPU backend {}", picked.display()));
        }
        Ok(())
    }

    fn ensure_backend_runtime(&self, needs_gpu_backends: bool) -> Result<()> {
        let mut state = llama_runtime_backend_state();

        if !state.backend_inited {
            unsafe { (self.llama_backend_init)() };
            state.backend_inited = true;
        }

        if needs_gpu_backends {
            if !state.gpu_backends_loaded {
                self.load_backends_gpu_allowed()?;
                state.gpu_backends_loaded = true;
                state.cpu_backends_loaded = true;
            }
        } else if !state.cpu_backends_loaded {
            self.load_backends_cpu_only()?;
            state.cpu_backends_loaded = true;
        }

        Ok(())
    }

    pub fn generate_chat(
        &self,
        model_path: &Path,
        system: &str,
        user: &str,
        max_tokens: usize,
        temp: f32,
        top_p: f32,
        top_k: i32,
        cancel: &std::sync::atomic::AtomicBool,
        mut on_token: impl FnMut(&str),
    ) -> Result<()> {
        // llama.cpp backend init/free is process-global. Serialize runtime use to avoid
        // concurrency issues between the orchestrator, bookkeeper, and AI-powered modules.
        let _lock = llama_runtime_lock();

        // Try GPU/Vulkan first, then a safer partial-offload, then fall back to CPU-only.
        // This keeps the app usable when Vulkan runs out of device memory.
        const SAFE_GPU_LAYERS: i32 = 16;

        let mut try_gpu_all = || {
            self.generate_chat_with_backend(
                BackendMode::GpuAllowed { n_gpu_layers: -1 },
                ContextProfile::Default,
                model_path,
                system,
                user,
                max_tokens,
                temp,
                top_p,
                top_k,
                cancel,
                &mut on_token,
            )
        };

        match try_gpu_all() {
            Ok(()) => Ok(()),
            Err(gpu_err) => {
                on_token(
                    "\n\n[Runtime] Vulkan full-offload failed; trying reduced GPU layers...\n\n",
                );
                let try_gpu_safe = self.generate_chat_with_backend(
                    BackendMode::GpuAllowed {
                        n_gpu_layers: SAFE_GPU_LAYERS,
                    },
                    ContextProfile::Safe,
                    model_path,
                    system,
                    user,
                    max_tokens,
                    temp,
                    top_p,
                    top_k,
                    cancel,
                    &mut on_token,
                );

                match try_gpu_safe {
                    Ok(()) => Ok(()),
                    Err(gpu_safe_err) => {
                        on_token(
                            "\n\n[Runtime] Vulkan reduced-offload failed; falling back to CPU-only...\n\n",
                        );
                        let cpu_res = self.generate_chat_with_backend(
                            BackendMode::CpuOnly,
                            ContextProfile::Default,
                            model_path,
                            system,
                            user,
                            max_tokens,
                            temp,
                            top_p,
                            top_k,
                            cancel,
                            &mut on_token,
                        );
                        match cpu_res {
                            Ok(()) => Ok(()),
                            Err(cpu_err) => {
                                on_token(
                                    "\n\n[Runtime] CPU fallback failed; trying CPU safe-mode...\n\n",
                                );
                                let cpu_safe_res = self.generate_chat_with_backend(
                                    BackendMode::CpuOnly,
                                    ContextProfile::Safe,
                                    model_path,
                                    system,
                                    user,
                                    max_tokens,
                                    temp,
                                    top_p,
                                    top_k,
                                    cancel,
                                    &mut on_token,
                                );
                                match cpu_safe_res {
                                    Ok(()) => Ok(()),
                                    Err(cpu_safe_err) => Err(anyhow!(
                                        "GPU attempt failed: {gpu_err:#}\nReduced-GPU attempt failed: {gpu_safe_err:#}\nCPU fallback also failed: {cpu_err:#}\nCPU safe-mode also failed: {cpu_safe_err:#}"
                                    )),
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn load_model(&self, model_path: &Path, n_gpu_layers: i32) -> Result<*mut llama_model> {
        let _ = read_gguf_architecture(model_path)
            .with_context(|| format!("read GGUF metadata for {}", model_path.display()))?;

        let mut params = unsafe { (self.llama_model_default_params)() };
        params.n_gpu_layers = n_gpu_layers;
        params.use_mmap = true;
        params.use_mlock = false;

        let cpath = CString::new(model_path.to_string_lossy().as_bytes().to_vec())
            .context("model path contains NUL")?;
        let model = unsafe { (self.llama_model_load_from_file)(cpath.as_ptr(), params) };
        if model.is_null() {
            return Err(anyhow!("failed to load model {}", model_path.display()));
        }
        Ok(model)
    }

    fn generate_chat_with_backend(
        &self,
        backend: BackendMode,
        profile: ContextProfile,
        model_path: &Path,
        system: &str,
        user: &str,
        max_tokens: usize,
        temp: f32,
        top_p: f32,
        top_k: i32,
        cancel: &std::sync::atomic::AtomicBool,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<()> {
        self.ensure_backend_runtime(matches!(backend, BackendMode::GpuAllowed { .. }))?;

        let n_gpu_layers = match backend {
            BackendMode::GpuAllowed { n_gpu_layers } => n_gpu_layers,
            BackendMode::CpuOnly => 0,
        };

        let model = self.load_model(model_path, n_gpu_layers)?;
        struct ModelFreeGuard<'a> {
            llama: &'a Llama,
            model: *mut llama_model,
        }
        impl Drop for ModelFreeGuard<'_> {
            fn drop(&mut self) {
                unsafe { (self.llama.llama_model_free)(self.model) };
            }
        }
        let _mguard = ModelFreeGuard { llama: self, model };

        let ctx = self.create_context(model, backend, profile, cancel)?;
        struct CtxFreeGuard<'a> {
            llama: &'a Llama,
            ctx: *mut llama_context,
        }
        impl Drop for CtxFreeGuard<'_> {
            fn drop(&mut self) {
                unsafe { (self.llama.llama_free)(self.ctx) };
            }
        }
        let _cguard = CtxFreeGuard { llama: self, ctx };

        let vocab = unsafe { (self.llama_model_get_vocab)(model) };
        if vocab.is_null() {
            return Err(anyhow!("llama_model_get_vocab returned NULL"));
        }

        let prompt = self.build_prompt(model, system, user)?;
        let mut prompt_tokens = self.tokenize(vocab, &prompt, true)?;
        let effective_max_tokens =
            self.fit_prompt_tokens_to_context(ctx, &mut prompt_tokens, max_tokens, on_token)?;

        let mut n_past: llama_pos = 0;
        self.eval_chunked(ctx, &prompt_tokens, n_past, 512)?;
        n_past += prompt_tokens.len() as llama_pos;

        let sampler = self.create_sampler(temp, top_p, top_k)?;
        struct SamplerFreeGuard<'a> {
            llama: &'a Llama,
            sampler: *mut llama_sampler,
        }
        impl Drop for SamplerFreeGuard<'_> {
            fn drop(&mut self) {
                unsafe { (self.llama.llama_sampler_free)(self.sampler) };
            }
        }
        let _sguard = SamplerFreeGuard {
            llama: self,
            sampler,
        };

        // Generate tokens one by one.
        // Sample from the logits of the last token in the last decoded batch (llama.h recommends idx = -1).
        for _ in 0..effective_max_tokens {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            let token = unsafe { (self.llama_sampler_sample)(sampler, ctx, -1) };
            unsafe { (self.llama_sampler_accept)(sampler, token) };

            if unsafe { (self.llama_vocab_is_eog)(vocab, token) } {
                break;
            }

            if let Ok(piece) = self.token_to_piece(vocab, token) {
                on_token(&piece);
            }

            self.eval_with_pos(ctx, &[token], n_past)?;
            n_past += 1;
        }

        Ok(())
    }

    pub fn embed_text_cpu_only(&self, model_path: &Path, text: &str) -> Result<Vec<f32>> {
        let _lock = llama_runtime_lock();
        self.ensure_backend_runtime(false)?;

        let mut mparams = unsafe { (self.llama_model_default_params)() };
        mparams.n_gpu_layers = 0;
        mparams.use_mmap = true;
        mparams.use_mlock = false;

        let cpath = CString::new(model_path.to_string_lossy().as_bytes().to_vec())
            .context("model path contains NUL")?;
        let model = unsafe { (self.llama_model_load_from_file)(cpath.as_ptr(), mparams) };
        if model.is_null() {
            return Err(anyhow!("failed to load model {}", model_path.display()));
        }
        struct ModelFreeGuard<'a> {
            llama: &'a Llama,
            model: *mut llama_model,
        }
        impl Drop for ModelFreeGuard<'_> {
            fn drop(&mut self) {
                unsafe { (self.llama.llama_model_free)(self.model) };
            }
        }
        let _mguard = ModelFreeGuard { llama: self, model };

        let mut cparams = unsafe { (self.llama_context_default_params)() };
        cparams.embeddings = true;
        if cparams.n_ctx == 0 {
            cparams.n_ctx = 512;
        }
        if cparams.n_batch == 0 {
            cparams.n_batch = 512;
        }
        if cparams.n_ubatch == 0 {
            cparams.n_ubatch = cparams.n_batch;
        }
        if cparams.n_batch > cparams.n_ctx {
            cparams.n_batch = cparams.n_ctx.max(1);
        }
        // Safer defaults for CPU-only embedding runs.
        cparams.flash_attn_type = llama_flash_attn_type_LLAMA_FLASH_ATTN_TYPE_DISABLED;
        cparams.offload_kqv = false;
        cparams.op_offload = false;
        cparams.no_perf = true;

        let threads = std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(1);
        if cparams.n_threads <= 0 {
            cparams.n_threads = threads;
        }
        if cparams.n_threads_batch <= 0 {
            cparams.n_threads_batch = threads;
        }

        let ctx = unsafe { (self.llama_init_from_model)(model, cparams) };
        if ctx.is_null() {
            return Err(anyhow!("failed to create context for embeddings"));
        }
        struct CtxFreeGuard<'a> {
            llama: &'a Llama,
            ctx: *mut llama_context,
        }
        impl Drop for CtxFreeGuard<'_> {
            fn drop(&mut self) {
                unsafe { (self.llama.llama_free)(self.ctx) };
            }
        }
        let _cguard = CtxFreeGuard { llama: self, ctx };

        unsafe { (self.llama_set_embeddings)(ctx, true) };

        let vocab = unsafe { (self.llama_model_get_vocab)(model) };
        if vocab.is_null() {
            return Err(anyhow!("llama_model_get_vocab returned NULL"));
        }

        let tokens = self.tokenize(vocab, text, true)?;
        self.eval_chunked(ctx, &tokens, 0, 512)?;

        let n_embd = unsafe { (self.llama_model_n_embd)(model) };
        if n_embd <= 0 {
            return Err(anyhow!("invalid n_embd {n_embd}"));
        }

        let emb_ptr = unsafe { (self.llama_get_embeddings)(ctx) };
        if emb_ptr.is_null() {
            return Err(anyhow!("llama_get_embeddings returned NULL"));
        }

        let emb = unsafe { std::slice::from_raw_parts(emb_ptr, n_embd as usize) }.to_vec();

        Ok(emb)
    }

    pub fn generate_text_cpu_only(
        &self,
        model_path: &Path,
        prompt: &str,
        max_tokens: usize,
        temp: f32,
        top_p: f32,
        top_k: i32,
        cancel: &std::sync::atomic::AtomicBool,
        mut on_token: impl FnMut(&str),
    ) -> Result<()> {
        let _lock = llama_runtime_lock();
        self.ensure_backend_runtime(false)?;

        let mut mparams = unsafe { (self.llama_model_default_params)() };
        mparams.n_gpu_layers = 0;
        mparams.use_mmap = true;
        mparams.use_mlock = false;

        let cpath = CString::new(model_path.to_string_lossy().as_bytes().to_vec())
            .context("model path contains NUL")?;
        let model = unsafe { (self.llama_model_load_from_file)(cpath.as_ptr(), mparams) };
        if model.is_null() {
            return Err(anyhow!("failed to load model {}", model_path.display()));
        }
        struct ModelFreeGuard<'a> {
            llama: &'a Llama,
            model: *mut llama_model,
        }
        impl Drop for ModelFreeGuard<'_> {
            fn drop(&mut self) {
                unsafe { (self.llama.llama_model_free)(self.model) };
            }
        }
        let _mguard = ModelFreeGuard { llama: self, model };

        let mut cparams = unsafe { (self.llama_context_default_params)() };
        if cparams.n_ctx == 0 {
            cparams.n_ctx = 2048;
        }
        if cparams.n_batch == 0 {
            cparams.n_batch = 512;
        }
        if cparams.n_ubatch == 0 {
            cparams.n_ubatch = cparams.n_batch;
        }
        if cparams.n_batch > cparams.n_ctx {
            cparams.n_batch = cparams.n_ctx.max(1);
        }
        // Safer defaults for CPU-only helper generations (bookkeeper, etc).
        cparams.flash_attn_type = llama_flash_attn_type_LLAMA_FLASH_ATTN_TYPE_DISABLED;
        cparams.offload_kqv = false;
        cparams.op_offload = false;
        cparams.no_perf = true;
        cparams.abort_callback = Some(abort_callback);
        cparams.abort_callback_data = cancel as *const AtomicBool as *mut c_void;

        let threads = std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(1);
        if cparams.n_threads <= 0 {
            cparams.n_threads = threads;
        }
        if cparams.n_threads_batch <= 0 {
            cparams.n_threads_batch = threads;
        }

        let ctx = unsafe { (self.llama_init_from_model)(model, cparams) };
        if ctx.is_null() {
            return Err(anyhow!("failed to create context for generation"));
        }
        struct CtxFreeGuard<'a> {
            llama: &'a Llama,
            ctx: *mut llama_context,
        }
        impl Drop for CtxFreeGuard<'_> {
            fn drop(&mut self) {
                unsafe { (self.llama.llama_free)(self.ctx) };
            }
        }
        let _cguard = CtxFreeGuard { llama: self, ctx };

        let vocab = unsafe { (self.llama_model_get_vocab)(model) };
        if vocab.is_null() {
            return Err(anyhow!("llama_model_get_vocab returned NULL"));
        }

        let prompt_tokens = self.tokenize(vocab, prompt, true)?;
        let mut n_past: llama_pos = 0;
        self.eval_chunked(ctx, &prompt_tokens, n_past, 512)?;
        n_past += prompt_tokens.len() as llama_pos;

        let sampler = self.create_sampler(temp, top_p, top_k)?;
        struct SamplerFreeGuard<'a> {
            llama: &'a Llama,
            sampler: *mut llama_sampler,
        }
        impl Drop for SamplerFreeGuard<'_> {
            fn drop(&mut self) {
                unsafe { (self.llama.llama_sampler_free)(self.sampler) };
            }
        }
        let _sguard = SamplerFreeGuard {
            llama: self,
            sampler,
        };

        for _ in 0..max_tokens {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }

            // Sample from the logits of the last token in the last decoded batch (llama.h recommends idx = -1).
            let token = unsafe { (self.llama_sampler_sample)(sampler, ctx, -1) };
            unsafe { (self.llama_sampler_accept)(sampler, token) };

            if unsafe { (self.llama_vocab_is_eog)(vocab, token) } {
                break;
            }

            if let Ok(piece) = self.token_to_piece(vocab, token) {
                on_token(&piece);
            }

            self.eval_with_pos(ctx, &[token], n_past)?;
            n_past += 1;
        }

        Ok(())
    }

    fn create_context(
        &self,
        model: *mut llama_model,
        backend: BackendMode,
        profile: ContextProfile,
        cancel: &AtomicBool,
    ) -> Result<*mut llama_context> {
        let mut params = unsafe { (self.llama_context_default_params)() };
        // Conservative defaults; can be surfaced in Settings later. llama.cpp currently
        // defaults to 512 here, which is too small once the GUI injects memory/sandbox
        // context, so choose the app's desired chat context explicitly.
        let desired_ctx: u32 = match profile {
            ContextProfile::Default => 4096,
            ContextProfile::Safe => 2048,
        };
        let desired_batch: u32 = match profile {
            ContextProfile::Default => 512,
            ContextProfile::Safe => 256,
        };

        params.n_ctx = desired_ctx;
        params.n_batch = desired_batch;

        if params.n_ubatch == 0 {
            params.n_ubatch = params.n_batch;
        }
        if params.n_batch > params.n_ctx {
            params.n_batch = params.n_ctx.max(1);
        }
        if params.n_ubatch > params.n_batch {
            params.n_ubatch = params.n_batch;
        }
        params.no_perf = true;

        let threads = std::thread::available_parallelism()
            .map(|n| n.get() as i32)
            .unwrap_or(1);
        if params.n_threads <= 0 {
            params.n_threads = threads;
        }
        if params.n_threads_batch <= 0 {
            params.n_threads_batch = threads;
        }

        match backend {
            BackendMode::GpuAllowed { .. } => {
                if matches!(profile, ContextProfile::Safe) {
                    params.flash_attn_type = llama_flash_attn_type_LLAMA_FLASH_ATTN_TYPE_DISABLED;
                    // Reduce VRAM pressure in safe mode.
                    params.offload_kqv = false;
                    params.op_offload = false;
                }
            }
            BackendMode::CpuOnly => {
                params.flash_attn_type = llama_flash_attn_type_LLAMA_FLASH_ATTN_TYPE_DISABLED;
                params.offload_kqv = false;
                params.op_offload = false;
                params.abort_callback = Some(abort_callback);
                params.abort_callback_data = cancel as *const AtomicBool as *mut c_void;
            }
        }

        let ctx = unsafe { (self.llama_init_from_model)(model, params) };
        if ctx.is_null() {
            return Err(anyhow!("failed to create context"));
        }
        Ok(ctx)
    }

    fn build_prompt(&self, model: *const llama_model, system: &str, user: &str) -> Result<String> {
        let tmpl_ptr = unsafe { (self.llama_model_chat_template)(model) };
        let tmpl = if tmpl_ptr.is_null() {
            None
        } else {
            let s = unsafe { std::ffi::CStr::from_ptr(tmpl_ptr) }
                .to_string_lossy()
                .to_string();
            if s.trim().is_empty() { None } else { Some(s) }
        };

        if tmpl.is_none() {
            return Ok(format!("{system}\n\n{user}\n"));
        }

        let sys_c = CString::new(system).context("system message contains NUL")?;
        let user_c = CString::new(user).context("user message contains NUL")?;
        let role_system = CString::new("system")?;
        let role_user = CString::new("user")?;
        let chat = vec![
            llama_chat_message {
                role: role_system.as_ptr(),
                content: sys_c.as_ptr(),
            },
            llama_chat_message {
                role: role_user.as_ptr(),
                content: user_c.as_ptr(),
            },
        ];

        let tmpl_c = CString::new(tmpl.unwrap_or_default())?;
        let mut buf = vec![0u8; 8192];
        let n = unsafe {
            (self.llama_chat_apply_template)(
                tmpl_c.as_ptr(),
                chat.as_ptr(),
                chat.len(),
                true,
                buf.as_mut_ptr() as *mut c_char,
                buf.len() as c_int,
            )
        };
        if n < 0 {
            return Err(anyhow!("llama_chat_apply_template failed"));
        }
        if (n as usize) >= buf.len() {
            buf.resize(n as usize + 1, 0);
            let n2 = unsafe {
                (self.llama_chat_apply_template)(
                    tmpl_c.as_ptr(),
                    chat.as_ptr(),
                    chat.len(),
                    true,
                    buf.as_mut_ptr() as *mut c_char,
                    buf.len() as c_int,
                )
            };
            if n2 < 0 {
                return Err(anyhow!("llama_chat_apply_template failed (2nd pass)"));
            }
            buf.truncate(n2 as usize);
        } else {
            buf.truncate(n as usize);
        }

        Ok(String::from_utf8_lossy(&buf).to_string())
    }

    fn tokenize(
        &self,
        vocab: *const llama_vocab,
        text: &str,
        add_special: bool,
    ) -> Result<Vec<llama_token>> {
        let ctext = CString::new(text).context("prompt contains NUL")?;
        let mut tokens = vec![0 as llama_token; (text.len() as i32 + 32) as usize];
        let n = unsafe {
            (self.llama_tokenize)(
                vocab,
                ctext.as_ptr(),
                text.len() as c_int,
                tokens.as_mut_ptr(),
                tokens.len() as c_int,
                add_special,
                true,
            )
        };
        if n == i32::MIN {
            return Err(anyhow!("tokenization overflow"));
        }
        if n < 0 {
            let needed = (-n) as usize;
            tokens.resize(needed, 0);
            let n2 = unsafe {
                (self.llama_tokenize)(
                    vocab,
                    ctext.as_ptr(),
                    text.len() as c_int,
                    tokens.as_mut_ptr(),
                    tokens.len() as c_int,
                    add_special,
                    true,
                )
            };
            if n2 < 0 {
                return Err(anyhow!("tokenization failed"));
            }
            tokens.truncate(n2 as usize);
            return Ok(tokens);
        }
        tokens.truncate(n as usize);
        Ok(tokens)
    }

    fn token_to_piece(&self, vocab: *const llama_vocab, token: llama_token) -> Result<String> {
        let mut buf = vec![0u8; 256];
        let n = unsafe {
            (self.llama_token_to_piece)(
                vocab,
                token,
                buf.as_mut_ptr() as *mut c_char,
                buf.len() as c_int,
                0,
                true,
            )
        };
        if n < 0 {
            let needed = (-n) as usize;
            buf.resize(needed, 0);
            let n2 = unsafe {
                (self.llama_token_to_piece)(
                    vocab,
                    token,
                    buf.as_mut_ptr() as *mut c_char,
                    buf.len() as c_int,
                    0,
                    true,
                )
            };
            if n2 < 0 {
                return Err(anyhow!("token_to_piece failed"));
            }
            buf.truncate(n2 as usize);
        } else {
            buf.truncate(n as usize);
        }
        Ok(String::from_utf8_lossy(&buf).to_string())
    }

    fn eval_with_pos(
        &self,
        ctx: *mut llama_context,
        tokens: &[llama_token],
        start_pos: llama_pos,
    ) -> Result<()> {
        if tokens.is_empty() {
            return Ok(());
        }

        let mut batch = unsafe { (self.llama_batch_init)(tokens.len() as c_int, 0, 1) };
        if batch.token.is_null()
            || batch.pos.is_null()
            || batch.n_seq_id.is_null()
            || batch.seq_id.is_null()
            || batch.logits.is_null()
        {
            unsafe { (self.llama_batch_free)(batch) };
            return Err(anyhow!(
                "llama_batch_init returned incomplete batch buffers"
            ));
        }

        for (i, &t) in tokens.iter().enumerate() {
            unsafe {
                *batch.token.add(i) = t;
                *batch.pos.add(i) = start_pos + i as llama_pos;
                *batch.n_seq_id.add(i) = 1;
                *(*batch.seq_id.add(i)).add(0) = 0;
                *batch.logits.add(i) = if i == tokens.len() - 1 { 1 } else { 0 };
            }
        }
        batch.n_tokens = tokens.len() as c_int;

        let rc = unsafe { (self.llama_decode)(ctx, batch) };
        unsafe { (self.llama_batch_free)(batch) };
        if rc != 0 {
            return Err(anyhow!("llama_decode failed: {rc}"));
        }
        Ok(())
    }

    fn eval_chunked(
        &self,
        ctx: *mut llama_context,
        tokens: &[llama_token],
        start_pos: llama_pos,
        max_batch_tokens: usize,
    ) -> Result<()> {
        if tokens.is_empty() {
            return Ok(());
        }

        let ctx_batch = unsafe { (self.llama_n_batch)(ctx.cast_const()) } as usize;
        let max_batch_tokens = max_batch_tokens.max(1).min(ctx_batch.max(1));
        let mut pos = start_pos;
        for chunk in tokens.chunks(max_batch_tokens) {
            self.eval_with_pos(ctx, chunk, pos)?;
            pos += chunk.len() as llama_pos;
        }
        Ok(())
    }

    fn fit_prompt_tokens_to_context(
        &self,
        ctx: *const llama_context,
        prompt_tokens: &mut Vec<llama_token>,
        max_tokens: usize,
        on_token: &mut dyn FnMut(&str),
    ) -> Result<usize> {
        let n_ctx = unsafe { (self.llama_n_ctx)(ctx) } as usize;
        if n_ctx == 0 {
            return Ok(max_tokens);
        }

        let generation_reserve = max_tokens.min(n_ctx.saturating_sub(1)).max(1);
        let usable_prompt_tokens = n_ctx.saturating_sub(generation_reserve).max(1);
        if prompt_tokens.len() <= usable_prompt_tokens {
            let available_generation_tokens = n_ctx.saturating_sub(prompt_tokens.len()).max(1);
            return Ok(max_tokens.min(available_generation_tokens));
        }

        let dropped = prompt_tokens.len() - usable_prompt_tokens;
        prompt_tokens.drain(..dropped);
        on_token(&format!(
            "\n\n[Runtime] Prompt context was too large for this run; trimmed {dropped} oldest prompt tokens before generation.\n\n"
        ));
        let available_generation_tokens = n_ctx.saturating_sub(prompt_tokens.len()).max(1);
        let effective_max_tokens = max_tokens.min(available_generation_tokens);
        if effective_max_tokens < max_tokens {
            on_token(&format!(
                "\n\n[Runtime] Output capped at {effective_max_tokens} tokens for this context window.\n\n"
            ));
        }
        Ok(effective_max_tokens)
    }

    fn create_sampler(&self, temp: f32, top_p: f32, top_k: i32) -> Result<*mut llama_sampler> {
        let params = unsafe { (self.llama_sampler_chain_default_params)() };
        let chain = unsafe { (self.llama_sampler_chain_init)(params) };
        if chain.is_null() {
            return Err(anyhow!("failed to create sampler chain"));
        }
        let k = top_k.clamp(0, i32::MAX);
        let p = top_p.clamp(0.0, 1.0);
        let t = temp.max(0.0);
        unsafe {
            (self.llama_sampler_chain_add)(chain, (self.llama_sampler_init_top_k)(k));
            (self.llama_sampler_chain_add)(chain, (self.llama_sampler_init_top_p)(p, 1));
            (self.llama_sampler_chain_add)(chain, (self.llama_sampler_init_temp)(t));
            if t <= 0.0 {
                (self.llama_sampler_chain_add)(chain, (self.llama_sampler_init_greedy)());
            } else {
                let seed = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| (d.as_nanos() as u64) ^ (std::process::id() as u64))
                    .unwrap_or(0) as u32;
                (self.llama_sampler_chain_add)(chain, (self.llama_sampler_init_dist)(seed));
            }
        }
        Ok(chain)
    }
}

impl Drop for Llama {
    fn drop(&mut self) {
        let mut state = llama_runtime_backend_state();
        state.live_runtime_handles = state.live_runtime_handles.saturating_sub(1);
        if state.live_runtime_handles == 0 {
            state.gpu_backends_loaded = false;
            state.cpu_backends_loaded = false;
            state.backend_inited = false;
        }
    }
}

fn prepend_to_path(dir: &Path) -> Result<()> {
    let dir = dir
        .canonicalize()
        .with_context(|| format!("canonicalize {}", dir.display()))?;
    let key = "PATH";
    let current = std::env::var_os(key).unwrap_or_default();
    let mut parts: Vec<PathBuf> = std::env::split_paths(&current).collect();
    if !parts.iter().any(|p| p == &dir) {
        parts.insert(0, dir);
        let new_val = std::env::join_paths(parts).context("join PATH")?;
        unsafe { std::env::set_var(key, &new_val) };
    }
    Ok(())
}
