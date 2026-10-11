//! Hand-written OpenGL 3.3 bindings (only what this program calls).
#![allow(non_snake_case, non_upper_case_globals)]

use crate::win32::get_proc;
use std::ffi::c_void;

pub type GLenum = u32;
pub type GLuint = u32;
pub type GLint = i32;
pub type GLsizei = i32;
pub type GLfloat = f32;

pub const COLOR_BUFFER_BIT: u32 = 0x4000;
pub const DEPTH_BUFFER_BIT: u32 = 0x0100;
pub const DEPTH_TEST: GLenum = 0x0B71;
pub const BLEND: GLenum = 0x0BE2;
pub const SRC_ALPHA: GLenum = 0x0302;
pub const ONE_MINUS_SRC_ALPHA: GLenum = 0x0303;
pub const TRIANGLES: GLenum = 0x0004;
pub const LINES: GLenum = 0x0001;
pub const CULL_FACE: GLenum = 0x0B44;
pub const BACK: GLenum = 0x0405;
pub const POLYGON_OFFSET_FILL: GLenum = 0x8037;
pub const RGBA32F: GLint = 0x8814;
pub const FLOAT: GLenum = 0x1406;
pub const UNSIGNED_BYTE: GLenum = 0x1401;
pub const ARRAY_BUFFER: GLenum = 0x8892;
pub const ELEMENT_ARRAY_BUFFER: GLenum = 0x8893;
pub const UNSIGNED_INT: GLenum = 0x1405;
pub const STATIC_DRAW: GLenum = 0x88E4;
pub const DYNAMIC_DRAW: GLenum = 0x88E8;
pub const VERTEX_SHADER: GLenum = 0x8B31;
pub const FRAGMENT_SHADER: GLenum = 0x8B30;
pub const COMPILE_STATUS: GLenum = 0x8B81;
pub const LINK_STATUS: GLenum = 0x8B82;
pub const INFO_LOG_LENGTH: GLenum = 0x8B84;
pub const TEXTURE_2D: GLenum = 0x0DE1;
pub const TEXTURE0: GLenum = 0x84C0;
pub const TEXTURE1: GLenum = 0x84C1;
pub const TEXTURE2: GLenum = 0x84C2;
pub const TEXTURE3: GLenum = 0x84C3;
pub const LINEAR: GLint = 0x2601;
pub const REPEAT: GLint = 0x2901;
pub const UNPACK_ALIGNMENT: GLenum = 0x0CF5;
pub const RGBA: GLenum = 0x1908;
pub const RGBA8: GLenum = 0x8058;
pub const TEXTURE_MAG_FILTER: GLenum = 0x2800;
pub const TEXTURE_MIN_FILTER: GLenum = 0x2801;
pub const TEXTURE_WRAP_S: GLenum = 0x2802;
pub const TEXTURE_WRAP_T: GLenum = 0x2803;
pub const NEAREST: GLint = 0x2600;
pub const CLAMP_TO_EDGE: GLint = 0x812F;
pub const SCISSOR_TEST: GLenum = 0x0C11;
pub const RED: GLenum = 0x1903;
pub const R8: GLint = 0x8229;

macro_rules! gl_fns {
    ($($name:ident : fn($($arg:ty),*) $(-> $ret:ty)?),* $(,)?) => {
        #[derive(Clone, Copy)]
        pub struct Gl { $(pub $name: unsafe extern "system" fn($($arg),*) $(-> $ret)?),* }
        impl Gl {
            /// # Safety
            /// A GL context must be current on this thread.
            pub unsafe fn load() -> Result<Gl, String> {
                Ok(Gl { $($name: {
                    let p = get_proc(concat!(stringify!($name), "\0"));
                    if p.is_null() { return Err(format!("missing GL function {}", stringify!($name))); }
                    unsafe { std::mem::transmute::<*const c_void, _>(p) }
                }),*})
            }
        }
    };
}

gl_fns! {
    glClearColor: fn(GLfloat, GLfloat, GLfloat, GLfloat),
    glClear: fn(u32),
    glViewport: fn(GLint, GLint, GLsizei, GLsizei),
    glEnable: fn(GLenum),
    glPixelStorei: fn(GLenum, GLint),
    glCullFace: fn(GLenum),
    glPolygonOffset: fn(GLfloat, GLfloat),
    glReadPixels: fn(GLint, GLint, GLsizei, GLsizei, GLenum, GLenum, *mut c_void),
    glScissor: fn(GLint, GLint, GLsizei, GLsizei),
    glCopyTexSubImage2D: fn(GLenum, GLint, GLint, GLint, GLint, GLint, GLsizei, GLsizei),
    glBlendFunc: fn(GLenum, GLenum),
    glDepthMask: fn(u8),
    glDisable: fn(GLenum),
    glUniform1f: fn(GLint, GLfloat),
    glDrawArrays: fn(GLenum, GLint, GLsizei),
    glDrawElements: fn(GLenum, GLsizei, GLenum, *const c_void),
    glGenTextures: fn(GLsizei, *mut GLuint),
    glBindTexture: fn(GLenum, GLuint),
    glTexImage2D: fn(GLenum, GLint, GLint, GLsizei, GLsizei, GLint, GLenum, GLenum, *const c_void),
    glTexParameteri: fn(GLenum, GLenum, GLint),
    glActiveTexture: fn(GLenum),
    glCreateShader: fn(GLenum) -> GLuint,
    glShaderSource: fn(GLuint, GLsizei, *const *const u8, *const GLint),
    glCompileShader: fn(GLuint),
    glGetShaderiv: fn(GLuint, GLenum, *mut GLint),
    glGetShaderInfoLog: fn(GLuint, GLsizei, *mut GLsizei, *mut u8),
    glCreateProgram: fn() -> GLuint,
    glAttachShader: fn(GLuint, GLuint),
    glLinkProgram: fn(GLuint),
    glGetProgramiv: fn(GLuint, GLenum, *mut GLint),
    glGetProgramInfoLog: fn(GLuint, GLsizei, *mut GLsizei, *mut u8),
    glUseProgram: fn(GLuint),
    glGetUniformLocation: fn(GLuint, *const u8) -> GLint,
    glUniformMatrix4fv: fn(GLint, GLsizei, u8, *const GLfloat),
    glUniform1i: fn(GLint, GLint),
    glUniform2f: fn(GLint, GLfloat, GLfloat),
    glUniform4f: fn(GLint, GLfloat, GLfloat, GLfloat, GLfloat),
    glUniform4fv: fn(GLint, GLsizei, *const GLfloat),
    glUniform3f: fn(GLint, GLfloat, GLfloat, GLfloat),
    glGenVertexArrays: fn(GLsizei, *mut GLuint),
    glBindVertexArray: fn(GLuint),
    glGenBuffers: fn(GLsizei, *mut GLuint),
    glBindBuffer: fn(GLenum, GLuint),
    glBufferData: fn(GLenum, isize, *const c_void, GLenum),
    glVertexAttribPointer: fn(GLuint, GLint, GLenum, u8, GLsizei, *const c_void),
    glEnableVertexAttribArray: fn(GLuint),
}

impl Gl {
    pub unsafe fn compile_program(&self, vs: &str, fs: &str) -> Result<GLuint, String> {
        unsafe {
            let prog = (self.glCreateProgram)();
            for (kind, src) in [(VERTEX_SHADER, vs), (FRAGMENT_SHADER, fs)] {
                let sh = (self.glCreateShader)(kind);
                let ptr = src.as_ptr();
                let len = src.len() as GLint;
                (self.glShaderSource)(sh, 1, &ptr, &len);
                (self.glCompileShader)(sh);
                let mut ok = 0;
                (self.glGetShaderiv)(sh, COMPILE_STATUS, &mut ok);
                if ok == 0 {
                    let mut n = 0;
                    (self.glGetShaderiv)(sh, INFO_LOG_LENGTH, &mut n);
                    let mut buf = vec![0u8; n.max(1) as usize];
                    (self.glGetShaderInfoLog)(sh, n, std::ptr::null_mut(), buf.as_mut_ptr());
                    return Err(format!("shader compile error:\n{}", String::from_utf8_lossy(&buf)));
                }
                (self.glAttachShader)(prog, sh);
            }
            (self.glLinkProgram)(prog);
            let mut ok = 0;
            (self.glGetProgramiv)(prog, LINK_STATUS, &mut ok);
            if ok == 0 {
                let mut n = 0;
                (self.glGetProgramiv)(prog, INFO_LOG_LENGTH, &mut n);
                let mut buf = vec![0u8; n.max(1) as usize];
                (self.glGetProgramInfoLog)(prog, n, std::ptr::null_mut(), buf.as_mut_ptr());
                return Err(format!("program link error:\n{}", String::from_utf8_lossy(&buf)));
            }
            Ok(prog)
        }
    }
}
