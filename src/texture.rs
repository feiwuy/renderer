use crate::{context, gl};

pub struct Texture {
    pub tid: gl::types::GLuint,
}

impl Texture {
    pub fn new(_ctx: &context::Context, p: &str) -> Self {
        let img = image::open(p).expect("贴图加载失败").to_rgba8();
        let (w, h) = (img.width() as i32, img.height() as i32);
        let pixels = img.as_raw();
        let mut texture: gl::types::GLuint = 0;
        unsafe {
            gl::GenTextures(1, &mut texture as *mut gl::types::GLuint);
            gl::BindTexture(gl::TEXTURE_2D, texture);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
            gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA as i32,
                w,
                h,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                pixels.as_ptr() as *const std::ffi::c_void,
            );
            gl::GenerateMipmap(gl::TEXTURE_2D);
        }
        Self { tid: texture }
    }

    pub fn reload(&self, _ctx: &context::Context, p: &str) {
        let img = image::open(p).expect("贴图加载失败").to_rgba8();
        let (w, h) = (img.width() as i32, img.height() as i32);
        let pixels = img.as_raw();
        unsafe {
            gl::BindTexture(gl::TEXTURE_2D, self.tid);
            gl::TexImage2D(
                gl::TEXTURE_2D,
                0,
                gl::RGBA as i32,
                w,
                h,
                0,
                gl::RGBA,
                gl::UNSIGNED_BYTE,
                pixels.as_ptr() as *const std::ffi::c_void,
            );
            gl::GenerateMipmap(gl::TEXTURE_2D);
        }
    }

    pub fn bind(&self) {
        unsafe {
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, self.tid);
        }
    }
}
