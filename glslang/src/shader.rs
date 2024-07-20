use crate::ctypes::{ResourceType, ShaderOptions, ShaderStage};
use crate::error::GlslangError;
use crate::error::GlslangError::ParseError;
use crate::include::IncludeHandler;
use crate::{include, limits, limits::ResourceLimits, Compiler};
use glslang_sys as sys;
use glslang_sys::glsl_include_callbacks_s;
use std::collections::HashMap;
use std::ffi::{c_void, CStr, CString};
use std::ops::BitOr;
use std::ptr::NonNull;

use crate::{GlslProfile, SourceLanguage, SpirvVersion};

/// A handle to a shader in the glslang compiler.
pub struct Shader<'a> {
    pub(crate) handle: NonNull<sys::glslang_shader_t>,
    pub(crate) stage: ShaderStage,
    pub(crate) is_spirv: bool,
    _compiler: &'a Compiler,
}

impl<'a> Shader<'a> {
    /// Create a new shader instance with the provided [`ShaderInput`](crate::ShaderInput).
    pub fn new(_compiler: &'a Compiler, input: ShaderInput) -> Result<Self, GlslangError> {
        let shader = Self {
            handle: unsafe {
                NonNull::new(sys::glslang_shader_create(&input.input))
                    .expect("glslang created null shader")
            },
            stage: input.input.stage,
            is_spirv: input.input.target_language == sys::glslang_target_language_t::SPIRV,
            _compiler,
        };

        let preamble = input.defines
            .iter()
            .map(|(k, v)| format!("#define {} {}\n", k, v.clone().unwrap_or_default()))
            .collect::<Vec<String>>()
            .join("");

        let cpreamble = CString::new(preamble).expect("Invalid preamble format");
        unsafe {
            sys::glslang_shader_set_preamble(shader.handle.as_ptr(), cpreamble.as_ptr());
        }

        unsafe {
            if sys::glslang_shader_preprocess(shader.handle.as_ptr(), &input.input) == 0 {
                return Err(ParseError(shader.get_log()));
            }
        }

        unsafe {
            if sys::glslang_shader_parse(shader.handle.as_ptr(), &input.input) == 0 {
                return Err(ParseError(shader.get_log()));
            }
        }
        Ok(shader)
    }

    /// Set shader options flags.
    pub fn options(&mut self, options: ShaderOptions) {
        unsafe { sys::glslang_shader_set_options(self.handle.as_ptr(), options.0) }
    }

    /// Shift the binding of the given resource type.
    /// This doesn't actually seem to do anything and has the potential for unsoundness.
    #[doc(hidden)]
    #[allow(unused)]
    fn shift_binding(&mut self, resource_type: ResourceType, base: u32) {
        unsafe {
            sys::glslang_shader_shift_binding(self.handle.as_ptr(), resource_type, base);
        }
    }

    /// Shift the binding of the given resource type to the specified base and descriptor set.
    /// This doesn't actually seem to do anything and has the potential for unsoundness.
    #[doc(hidden)]
    #[allow(unused)]
    fn shift_binding_for_set(&mut self, resource_type: ResourceType, base: u32, set: u32) {
        unsafe {
            sys::glslang_shader_shift_binding_for_set(
                self.handle.as_ptr(),
                resource_type,
                base,
                set,
            );
        }
    }

    /// Set the GLSL version of the shader
    /// This doesn't actually seem to do anything and has the potential for unsoundness.
    #[doc(hidden)]
    #[allow(unused)]
    fn glsl_version(&mut self, version: i32) {
        unsafe { sys::glslang_shader_set_glsl_version(self.handle.as_ptr(), version) }
    }

    fn get_log(&self) -> String {
        let c_str =
            unsafe { CStr::from_ptr(sys::glslang_shader_get_info_log(self.handle.as_ptr())) };

        let string = CString::from(c_str)
            .into_string()
            .expect("Expected glslang info log to be valid UTF-8");

        string
    }

    /// Convenience method to compile this shader without linking to other shaders.
    pub fn compile(&self) -> Result<Vec<u32>, GlslangError> {
        let mut program = self._compiler.create_program();
        program.add_shader(&self);
        program.compile(self.stage)
    }

    /// Convenience method to compile this shader without linking to other shaders, optimizing for size.
    pub fn compile_size_optimized(&self) -> Result<Vec<u32>, GlslangError> {
        let mut program = self._compiler.create_program();
        program.add_shader(&self);
        program.compile_size_optimized(self.stage)
    }

    /// Get the preprocessed shader string.
    pub fn get_preprocessed_code(&self) -> String {
        let c_str = unsafe {
            // SAFETY: for Shader to be initialized preprocessing + parsing had to be complete.
            CStr::from_ptr(sys::glslang_shader_get_preprocessed_code(
                self.handle.as_ptr(),
            ))
        };

        let string = CString::from(c_str)
            .into_string()
            .expect("Expected glslang info log to be valid UTF-8");

        string
    }
}

impl<'a> Drop for Shader<'a> {
    fn drop(&mut self) {
        unsafe { sys::glslang_shader_delete(self.handle.as_ptr()) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ctypes::ShaderStage;
    use crate::shader::{CompilerOptions, ShaderSource};

    #[test]
    pub fn test_parse() {
        let compiler = Compiler::acquire().unwrap();

        let source = ShaderSource::try_from(String::from(
            r#"
#version 450

layout(location = 0) out vec4 color;
layout(binding = 1) uniform sampler2D tex;

void main() {
    color = texture(tex, vec2(0.0));
}
        "#,
        ))
        .expect("source");

        let input = ShaderInput::new(
            &source,
            ShaderStage::Fragment,
            &CompilerOptions::default(),
            &[],
            None,
        )
        .expect("target");
        let shader = Shader::new(&compiler, input).expect("shader init");

        let code = shader.get_preprocessed_code();

        println!("{}", code);
    }
}

/// The source string of a shader.
#[derive(Debug, Clone)]
pub struct ShaderSource(CString);

impl From<String> for ShaderSource {
    fn from(value: String) -> Self {
        // panic safety: String never has null bytes
        Self(CString::new(value).unwrap())
    }
}

impl From<&str> for ShaderSource {
    fn from(value: &str) -> Self {
        // panic safety: String never has null bytes
        Self(CString::new(value.to_string()).unwrap())
    }
}

impl ShaderSource {
    pub fn parse_profile(&self) -> Option<(i32, GlslProfile)> {
        let Ok(string) = self.0.to_str() else {
            return None;
        };

        let Some(string) = string.trim().lines().next() else {
            return None;
        };

        let string = string.trim();
        if !string.starts_with("#version ") {
            return None;
        };

        let string = string.trim_start_matches("#version ");
        if string.len() < 3 {
            return None;
        }
        let (version, profile) = string.split_at(3);
        let Ok(version) = str::parse::<i32>(version) else {
            return None;
        };

        let profile = profile.trim();
        let profile = match profile {
            "compatibility" => GlslProfile::Compatibility,
            "es" => GlslProfile::ES,
            "core" => GlslProfile::Core,
            "" => GlslProfile::None,
            _ => return None,
        };

        Some((version, profile))
    }
}

/// An input to a [`Shader`](crate::Shader).
#[derive(Clone)]
pub struct ShaderInput<'a> {
    // Keep these alive.
    _source: &'a ShaderSource,
    _resource: &'a sys::glslang_resource_t,
    pub(crate) defines: HashMap<String, Option<String>>,
    pub(crate) input: sys::glslang_input_t,
}

/// Vulkan version
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub enum VulkanVersion {
    Vulkan1_0,
    Vulkan1_1,
    Vulkan1_2,
    Vulkan1_3,
}

/// OpenGL Version
#[allow(non_camel_case_types)]
#[derive(Debug, Clone)]
pub enum OpenGlVersion {
    OpenGL4_5,
}

/// The target environment to compile or validate the input shader to.
///
/// If no SPIR-V version is specified, the shader will be unable to be compiled.
#[derive(Debug, Clone)]
pub enum Target {
    /// No specified environment.
    ///
    /// This environment can optionally include a SPIR-V version.
    None(Option<SpirvVersion>),
    /// Validate the shader against Vulkan semantics. Vulkan requires GLSL 140 or above.
    Vulkan {
        /// The Vulkan API version to validate against.
        version: VulkanVersion,
        /// The SPIR-V version to compile to.
        ///
        /// Vulkan requires a SPIR-V version to be specified.
        spirv_version: SpirvVersion,
    },
    /// Validate the shader against OpenGL semantics.
    OpenGL {
        /// The OpenGL version to validate against. Currently only OpenGL 4.5 is supported.
        version: OpenGlVersion,
        /// An optional SPIR-V version if targeting OpenGL SPIR-V. Requires GLSL 330 or above.
        spirv_version: Option<SpirvVersion>,
    },
}

impl Target {
    fn env(&self) -> sys::glslang_client_t {
        match self {
            Target::None(_) => sys::glslang_client_t::None,
            Target::Vulkan { .. } => sys::glslang_client_t::Vulkan,
            Target::OpenGL { .. } => sys::glslang_client_t::OpenGL,
        }
    }

    fn target_spirv(&self) -> sys::glslang_target_language_t {
        match self {
            Target::None(spirv_version) | Target::OpenGL { spirv_version, .. } => {
                if spirv_version.is_some() {
                    sys::glslang_target_language_t::SPIRV
                } else {
                    sys::glslang_target_language_t::None
                }
            }
            Target::Vulkan { .. } => sys::glslang_target_language_t::SPIRV,
        }
    }

    fn env_version(&self) -> sys::glslang_target_client_version_t {
        match self {
            // Doesn't matter.
            Target::None(_) => sys::glslang_target_client_version_t::OpenGL450,
            Target::Vulkan { version, .. } => match version {
                VulkanVersion::Vulkan1_0 => sys::glslang_target_client_version_t::Vulkan1_0,
                VulkanVersion::Vulkan1_1 => sys::glslang_target_client_version_t::Vulkan1_1,
                VulkanVersion::Vulkan1_2 => sys::glslang_target_client_version_t::Vulkan1_2,
                VulkanVersion::Vulkan1_3 => sys::glslang_target_client_version_t::Vulkan1_3,
            },
            Target::OpenGL { version, .. } => match version {
                OpenGlVersion::OpenGL4_5 => sys::glslang_target_client_version_t::OpenGL450,
            },
        }
    }

    fn spirv_version(&self) -> sys::glslang_target_language_version_t {
        match self {
            // Doesn't matter.
            Target::None(spirv_version) | Target::OpenGL { spirv_version, .. } => {
                spirv_version.unwrap_or(sys::glslang_target_language_version_t::SPIRV1_0)
            }
            Target::Vulkan { spirv_version, .. } => *spirv_version,
        }
    }

    fn verify_glsl_profile(
        &self,
        profile: Option<&(i32, GlslProfile)>,
    ) -> Result<(), GlslangError> {
        let Some(&(version, profile)) = profile else {
            return Ok(());
        };

        // only version 300, 310, 320 is supported for ES
        if profile == GlslProfile::ES && version != 300 && version != 310 && version != 320 {
            return Err(GlslangError::VersionUnsupported(
                version,
                GlslProfile::ES,
            ));
        }

        if !matches!(version,
            100 | 110 | 120 | 130 | 140 | 150 | 300 | 310 | 320 | 330 | 400 | 410 | 420 | 430 | 440 | 450 | 460
        ) {
            return Err(GlslangError::VersionUnsupported(version, profile))
        }

        match self {
            Target::None(spirv_version) => {
                if spirv_version.is_some() && profile == GlslProfile::Compatibility {
                    return Err(GlslangError::InvalidProfile(
                        self.clone(),
                        version,
                        GlslProfile::Compatibility,
                    ));
                }
            }
            Target::Vulkan { .. } => {
                if version < 140 {
                    // Desktop shaders for Vulkan SPIR-V require version 140
                    return Err(GlslangError::InvalidProfile(self.clone(), version, profile));
                }

                // compilation for SPIR-V does not support the compatibility profile
                if profile == GlslProfile::Compatibility {
                    return Err(GlslangError::InvalidProfile(
                        self.clone(),
                        version,
                        GlslProfile::Compatibility,
                    ));
                }
            }
            Target::OpenGL { spirv_version, .. } => {
                if spirv_version.is_some() {
                    // OpenGL SPIRV needs 330+
                    if version < 330 {
                        return Err(GlslangError::InvalidProfile(self.clone(), version, profile));
                    }

                    // compilation for SPIR-V does not support the compatibility profile
                    if profile == GlslProfile::Compatibility {
                        return Err(GlslangError::InvalidProfile(
                            self.clone(),
                            version,
                            GlslProfile::Compatibility,
                        ));
                    }
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct ShaderMessage(pub i32);

impl ShaderMessage {    
    pub const DEFAULT: ShaderMessage = ShaderMessage(sys::glslang_messages_t::DEFAULT.0);
    pub const RELAXED_ERRORS: ShaderMessage = ShaderMessage(sys::glslang_messages_t::RELAXED_ERRORS.0);
    pub const SUPPRESS_WARNINGS: ShaderMessage = ShaderMessage(sys::glslang_messages_t::SUPPRESS_WARNINGS.0);
    pub const AST: ShaderMessage = ShaderMessage(sys::glslang_messages_t::AST.0);
    pub const SPV_RULES: ShaderMessage = ShaderMessage(sys::glslang_messages_t::SPV_RULES.0);
    pub const VULKAN_RULES: ShaderMessage = ShaderMessage(sys::glslang_messages_t::VULKAN_RULES.0);
    pub const ONLY_PREPROCESSOR: ShaderMessage = ShaderMessage(sys::glslang_messages_t::ONLY_PREPROCESSOR.0);
    pub const READ_HLSL: ShaderMessage = ShaderMessage(sys::glslang_messages_t::READ_HLSL.0);
    pub const CASCADING_ERRORS: ShaderMessage = ShaderMessage(sys::glslang_messages_t::CASCADING_ERRORS.0);
    pub const KEEP_UNCALLED: ShaderMessage = ShaderMessage(sys::glslang_messages_t::KEEP_UNCALLED.0);
    pub const HLSL_OFFSETS: ShaderMessage = ShaderMessage(sys::glslang_messages_t::HLSL_OFFSETS.0);
    pub const DEBUG_INFO: ShaderMessage = ShaderMessage(sys::glslang_messages_t::DEBUG_INFO.0);
    pub const HLSL_ENABLE_16BIT_TYPES: ShaderMessage = ShaderMessage(sys::glslang_messages_t::HLSL_ENABLE_16BIT_TYPES.0);
    pub const HLSL_LEGALIZATION: ShaderMessage = ShaderMessage(sys::glslang_messages_t::HLSL_LEGALIZATION.0);
    pub const HLSL_DX9_COMPATIBLE: ShaderMessage = ShaderMessage(sys::glslang_messages_t::HLSL_DX9_COMPATIBLE.0);
    pub const BUILTIN_SYMBOL_TABLE: ShaderMessage = ShaderMessage(sys::glslang_messages_t::BUILTIN_SYMBOL_TABLE.0);
    pub const ENHANCED: ShaderMessage = ShaderMessage(sys::glslang_messages_t::ENHANCED.0);
    pub const ABSOLUTE_PATH: ShaderMessage = ShaderMessage(sys::glslang_messages_t::ABSOLUTE_PATH.0);
    pub const DISPLAY_ERROR_COLUMN: ShaderMessage = ShaderMessage(sys::glslang_messages_t::DISPLAY_ERROR_COLUMN.0);

    pub fn convert(&self) -> sys::glslang_messages_t {
        sys::glslang_messages_t(self.0)
    }
}
impl BitOr for ShaderMessage {
    type Output = Self;

    // rhs is the "right-hand side" of the expression `a | b`
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// Options to configure the compilation of a shader.
#[derive(Debug, Clone)]
pub struct CompilerOptions {
    pub source_language: SourceLanguage,
    /// The target
    pub target: Target,
    /// The GLSL version profile.
    /// If specified, will force the specified profile on compilation.
    pub version_profile: Option<(i32, GlslProfile)>,
    // Messages to be passed to compiler
    pub messages: ShaderMessage,
}

impl Default for CompilerOptions {
    fn default() -> Self {
        Self {
            source_language: SourceLanguage::GLSL,
            target: Target::Vulkan {
                version: VulkanVersion::Vulkan1_0,
                spirv_version: SpirvVersion::SPIRV1_0,
            },
            version_profile: None,
            messages: ShaderMessage::DEFAULT
        }
    }
}

/// The input to a shader instance.
impl<'a> ShaderInput<'a> {
    /// Create a new [`ShaderInput`](crate::ShaderInput) with default limits.
    pub fn new(
        source: &'a ShaderSource,
        stage: ShaderStage,
        options: &CompilerOptions,
        defines: &[(&str, Option<&str>)],
        include_handler: Option<&'a mut dyn IncludeHandler>,
    ) -> Result<Self, GlslangError> {
        Self::new_with_limits(source, &limits::DEFAULT_LIMITS, stage, options, defines, include_handler)
    }


    /// Create a new [`ShaderInput`](crate::ShaderInput) with the specified resource limits.
    pub fn new_with_limits(
        source: &'a ShaderSource,
        resource: &'a ResourceLimits,
        stage: ShaderStage,
        options: &CompilerOptions,
        defines: &[(&str, Option<&str>)],
        include_handler: Option<&'a mut dyn IncludeHandler>,
    ) -> Result<Self, GlslangError> {
        let profile = options
            .version_profile
            .map_or_else(|| source.parse_profile(), |p| Some(p));

        if options.source_language == SourceLanguage::GLSL {
            options.target.verify_glsl_profile(profile.as_ref())?;
        }

        let callbacks_ctx = include_handler.map_or(core::ptr::null_mut(), |callback| {
            Box::into_raw(Box::new(callback))
        });
        
        Ok(Self {
            _source: source,
            _resource: &resource.0,
            defines: defines.iter().map(|v| (String::from(v.0), v.1.map(|s| s.to_string()))).collect(),
            input: sys::glslang_input_t {
                language: options.source_language,
                stage,
                client: options.target.env(),
                client_version: options.target.env_version(),
                target_language: options.target.target_spirv(),
                target_language_version: options.target.spirv_version(),
                code: source.0.as_ptr(),
                default_version: options.version_profile.map_or(100, |o| o.0),
                default_profile: options.version_profile.map_or(GlslProfile::None, |o| o.1),
                force_default_version_and_profile: options.version_profile.map_or(0, |_| 1),
                forward_compatible: 0,
                messages: options.messages.convert(),
                resource: &resource.0,
                callbacks: glsl_include_callbacks_s {
                    include_system: Some(include::_glslang_rs_sys_func),
                    include_local: Some(include::_glslang_rs_local_func),
                    free_include_result: Some(include::_glslang_rs_drop_result),
                },
                callbacks_ctx: callbacks_ctx as *mut c_void,
            },
        })
    }
}
