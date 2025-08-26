use std::{ffi::CStr, mem::MaybeUninit, sync::OnceLock};

mod ctypes;

/// Error types.
pub mod error;

/// Helpers for processing includes.
pub mod include;
/// Shader resouce limits.
pub mod limits;
mod program;
mod shader;

static COMPILER_INSTANCE: OnceLock<Option<Compiler>> = OnceLock::new();

/// A handle representing the glslang compiler instance.
pub struct Compiler;

pub struct Version {
    pub major: i32,
    pub minor: i32,
    pub patch: i32,
    pub flavor: &'static str
}

pub use crate::ctypes::*;

pub use program::Program;
pub use shader::*;

impl Compiler {
    /// Acquire a global instance of the compiler.
    pub fn acquire() -> Option<&'static Self> {
        COMPILER_INSTANCE
            .get_or_init(|| {
                unsafe {
                    if glslang_sys::glslang_initialize_process() == 0 {
                        return None;
                    }
                };
                Some(Self)
            })
            .as_ref()
    }

    pub fn version() -> Version {
        let mut version : MaybeUninit<glslang_sys::glslang_version_t> = MaybeUninit::uninit();
        let version = unsafe { 
            glslang_sys::glslang_get_version(version.as_mut_ptr());
            version.assume_init()
        };
        Version { 
            major: version.major, 
            minor: version.minor, 
            patch: version.patch, 
            flavor: unsafe { CStr::from_ptr(version.flavor).to_str().unwrap() }
        }
    }

    /// Create a [`Shader`](crate::Shader) with the given inputs.
    pub fn create_shader(&self, input: ShaderInput) -> Result<Shader, error::GlslangError> {
        Shader::new(&self, input)
    }

    /// Create a [`Program`](crate::Program) instance.
    pub fn create_program(&self) -> Program {
        Program::new(&self)
    }
}

impl Drop for Compiler {
    fn drop(&mut self) {
        unsafe {
            glslang_sys::glslang_finalize_process();
        }
    }
}
