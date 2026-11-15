use crate::{context, gl, shader, utils};

use std::{collections::HashMap, io::Read};

#[allow(dead_code)]
pub struct Primitive {
    pub vao: gl::types::GLuint,
    /// 这个图元在 mesh 局部空间的最高点 Y（用于按高度剔除）
    pub max_y: f32,
    /// 最低点 Y
    pub min_y: f32,
    pub mode: gl::types::GLenum,
    pub count: i32,
    pub index_type: gl::types::GLenum,
    pub index_offset: i32,
    pub material_index: usize,
}

#[allow(dead_code)]
pub struct Mesh {
    pub primitives: Vec<Primitive>,
}

#[allow(dead_code)]
pub struct Skin {
    pub inverse_bind_matrices: Vec<glam::Mat4>,
    pub joints: Vec<usize>,
}

#[allow(dead_code)]
pub struct Texture {
    pub tid: gl::types::GLuint,
}

impl Texture {
    pub fn bind(&self, _ctx: &context::Context) {
        unsafe {
            gl::ActiveTexture(gl::TEXTURE0);
            gl::BindTexture(gl::TEXTURE_2D, self.tid);
        }
    }
}

#[allow(dead_code)]
pub struct Material {
    /// 部位编号（1=脸/皮肤 2=头发 3=眼睛 4=衣服 9=其他），按材质名关键字判断
    pub part: u8,
    pub base_color_factor: glam::Vec4,
    pub base_color_texture: Option<Texture>,

    pub metallic_factor: f32,
    pub roughness_factor: f32,
    pub metallic_roughness_texture: Option<Texture>,

    pub normal_texture: Option<Texture>,

    pub occlusion_texture: Option<Texture>,

    pub emissive_factor: glam::Vec3,
    pub emissive_texture: Option<Texture>,
}

#[allow(dead_code)]
#[derive(Clone, PartialEq, Debug)]
pub struct Node {
    pub child_indices: Vec<usize>,
    pub mesh_index: Option<usize>,
    pub skin_index: Option<usize>,
    pub transform: glam::Mat4,
}

#[allow(dead_code)]
pub struct Scene {
    pub meshes: Vec<Mesh>,
    pub skins: Vec<Skin>,
    pub materials: Vec<Material>,
    pub nodes: Vec<Node>,
    pub bone_node_indices: HashMap<String, usize>,
    pub scene_node_indices: Vec<usize>,
}

/// 探测"胸部"的世界 Y 坐标：优先按骨骼名找 chest / upperChest / spine，找不到就退化成身高的 65%
pub fn detect_chest_y(path: &str) -> f32 {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return 0.0,
    };
    let gltf = match gltf::Gltf::from_slice(&data) {
        Ok(g) => g,
        Err(_) => return 0.0,
    };
    let doc = &gltf.document;

    // 父指针表
    let n = doc.nodes().len();
    let mut parent = vec![usize::MAX; n];
    for node in doc.nodes() {
        for ch in node.children() {
            parent[ch.index()] = node.index();
        }
    }
    let world_y = |idx: usize| -> f32 {
        let mut y = 0.0;
        let mut i = idx;
        loop {
            let node = doc.nodes().nth(i).unwrap();
            y += node.transform().decomposed().0[1];
            if parent[i] == usize::MAX {
                break;
            }
            i = parent[i];
        }
        y
    };

    // 按名字找骨骼（VRoid 导出的名字就是 chest / upperChest / spine）
    for want in ["upperchest", "chest", "spine"] {
        for node in doc.nodes() {
            if node.name().map(|nm| nm.to_lowercase()) == Some(want.to_string()) {
                let y = world_y(node.index());
                if y > 0.1 {
                    return y;
                }
            }
        }
    }

    // 退化方案：模型最高点 × 0.65
    let mut top = 0.0f32;
    for mesh in doc.meshes() {
        for prim in mesh.primitives() {
            if let Some(mx) = prim.get(&gltf::Semantic::Positions).and_then(|a| a.max()) {
                top = top.max(mx.get(1).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32);
            }
        }
    }
    top * 0.65
}

/// 轻量解析 GLB，只读 skins 的 joints 数量（不解码贴图/缓冲），
/// 用来决定 shader 里 joint_matrices 数组开多大
pub fn detect_max_joints(path: &str) -> usize {
    let data = match std::fs::read(path) {
        Ok(d) => d,
        Err(_) => return 0,
    };
    let gltf = match gltf::Gltf::from_slice(&data) {
        Ok(g) => g,
        Err(_) => return 0,
    };
    gltf.document
        .skins()
        .map(|s| s.joints().count())
        .max()
        .unwrap_or(0)
}

impl Scene {
    fn initialize_attrib(
        attrib: gl::types::GLuint,
        accessor: &gltf::Accessor,
        bufs: &Vec<(gl::types::GLuint, &gltf::buffer::Data)>,
    ) {
        let data_type = match accessor.data_type() {
            gltf::accessor::DataType::I8 => gl::BYTE,
            gltf::accessor::DataType::U8 => gl::UNSIGNED_BYTE,
            gltf::accessor::DataType::I16 => gl::SHORT,
            gltf::accessor::DataType::U16 => gl::UNSIGNED_SHORT,
            gltf::accessor::DataType::U32 => gl::UNSIGNED_INT,
            gltf::accessor::DataType::F32 => gl::FLOAT,
        };
        if let Some(_sparse) = accessor.sparse() {
            let get_buffer_data = |buffer: gltf::Buffer| bufs.get(buffer.index()).map(|x| &*x.1 .0);
            let iter =
                gltf::accessor::Iter::<[f32; 3]>::new(accessor.clone(), get_buffer_data).unwrap();
            let mut b: Vec<[f32; 3]> = Vec::new();
            for item in iter {
                b.push(item);
            }

            log::debug!(
                "attrib {}: length {}",
                attrib,
                (b.len() * accessor.dimensions().multiplicity() * accessor.data_type().size())
            );
            unsafe {
                let mut buf: gl::types::GLuint = 0;
                gl::GenBuffers(1, &mut buf as *mut gl::types::GLuint);
                gl::BindBuffer(gl::ARRAY_BUFFER, buf);
                gl::BufferData(
                    gl::ARRAY_BUFFER,
                    (b.len() * accessor.dimensions().multiplicity() * accessor.data_type().size())
                        as _,
                    b.as_ptr() as *const std::ffi::c_void,
                    gl::STATIC_DRAW,
                );
                gl::VertexAttribPointer(
                    attrib,
                    accessor.dimensions().multiplicity() as _,
                    data_type,
                    accessor.normalized() as _,
                    0,
                    0 as _,
                );
                gl::EnableVertexAttribArray(attrib);
            }
        } else {
            let view = accessor.view().unwrap();
            let buf = bufs.get(view.buffer().index()).unwrap().0;
            log::debug!("attrib {}: length {}", attrib, view.length());
            unsafe {
                gl::BindBuffer(gl::ARRAY_BUFFER, buf);
                gl::VertexAttribPointer(
                    attrib,
                    accessor.dimensions().multiplicity() as _,
                    data_type,
                    accessor.normalized() as _,
                    view.stride().unwrap_or_default() as _,
                    view.offset() as _,
                );
                gl::EnableVertexAttribArray(attrib);
            }
        }
    }

    pub fn new(ctx: &context::Context, path: &str) -> Self {
        let file = std::fs::File::open(path).unwrap();
        let mut reader = std::io::BufReader::new(file);
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).unwrap();
        let (gltf, buffers, images) = gltf::import_slice(bytes.as_slice()).unwrap();
        let json = gltf.clone().into_json();
        let vrm = json
            .extensions
            .expect("模型缺少 extensions")
            .vrmc_vrm
            .expect("这个模型不是 VRM 1.0（缺少 VRMC_vrm 扩展），本项目只支持 VRM 1.0");

        log::info!("specVersion: {}", vrm.spec_version);
        let mut max: i32 = 0;
        unsafe {
            gl::GetIntegerv(gl::MAX_VERTEX_ATTRIBS, &mut max as _);
        }
        log::info!("max: {}", max);

        // 只挂载模型里真实存在的表情；缺失的就不绑属性（对应 uniform 位置为 -1，上传时自动失效）
        let expressions: HashMap<
            String,
            (
                gltf::json::extensions::root::MorphTargetBind,
                gltf::Mesh,
                gl::types::GLuint,
            ),
        > = ctx
            .attrib_expressions
            .iter()
            .filter_map(|(enm, attrib)| {
                let preset = vrm.expressions.preset.get(enm)?;
                let expr = preset.morph_target_binds.first()?.clone();
                let mesh = gltf.nodes().nth(expr.node as _)?.mesh()?;
                Some((enm.clone(), (expr, mesh, *attrib)))
            })
            .collect();
        for missing in ctx
            .attrib_expressions
            .keys()
            .filter(|k| !expressions.contains_key(*k))
        {
            log::warn!("模型没有表情 {}，该表情不会有形变", missing);
        }

        let bufs: Vec<(gl::types::GLuint, &gltf::buffer::Data)> = buffers
            .iter()
            .map(|b| unsafe {
                let mut buf: gl::types::GLuint = 0;
                gl::GenBuffers(1, &mut buf as *mut gl::types::GLuint);
                gl::BindBuffer(gl::ARRAY_BUFFER, buf);
                gl::BufferData(
                    gl::ARRAY_BUFFER,
                    b.len() as _,
                    b.as_ptr() as *const std::ffi::c_void,
                    gl::STATIC_DRAW,
                );
                (buf, b)
            })
            .collect();

        // COLONQ_TEX_MAX=1024 之类：把过大的贴图降采样，内存占用能省一大截
        let tex_max: u32 = crate::utils::env_parse::<u32>("COLONQ_TEX_MAX").unwrap_or(0);
        if tex_max > 0 {
            log::info!("COLONQ_TEX_MAX={}：超过这个边长的贴图会被降采样", tex_max);
        }

        let tids: Vec<gl::types::GLuint> = images
            .iter()
            .map(|i| {
                // 需要时才做重采样（只处理 RGB/RGBA 8bit，其余格式原样上传）
                let resized: Option<(u32, u32, Vec<u8>)> = if tex_max > 0
                    && (i.width > tex_max || i.height > tex_max)
                    && matches!(
                        i.format,
                        gltf::image::Format::R8G8B8 | gltf::image::Format::R8G8B8A8
                    ) {
                    let scale = tex_max as f32 / i.width.max(i.height) as f32;
                    let nw = ((i.width as f32 * scale) as u32).max(1);
                    let nh = ((i.height as f32 * scale) as u32).max(1);
                    let buf = match i.format {
                        gltf::image::Format::R8G8B8 => {
                            image::RgbImage::from_raw(i.width, i.height, i.pixels.clone())
                                .map(|b| image::DynamicImage::ImageRgb8(b).to_rgba8())
                        }
                        _ => image::RgbaImage::from_raw(i.width, i.height, i.pixels.clone())
                            .map(|b| image::DynamicImage::ImageRgba8(b).to_rgba8()),
                    };
                    buf.map(|b| {
                        let r = image::imageops::resize(
                            &b,
                            nw,
                            nh,
                            image::imageops::FilterType::Triangle,
                        );
                        (nw, nh, r.into_raw())
                    })
                } else {
                    None
                };
                // (宽, 高, 通道格式, 数据类型, 数据指针)
                let (tw, th, fmt, ty, ptr): (u32, u32, u32, u32, *const std::ffi::c_void) =
                    match &resized {
                        Some((nw, nh, data)) => (
                            *nw,
                            *nh,
                            gl::RGBA,
                            gl::UNSIGNED_BYTE,
                            data.as_ptr() as *const std::ffi::c_void,
                        ),
                        None => (
                            i.width,
                            i.height,
                            match i.format {
                                gltf::image::Format::R8 => gl::RED,
                                gltf::image::Format::R8G8 => gl::RG,
                                gltf::image::Format::R8G8B8 => gl::RGB,
                                gltf::image::Format::R8G8B8A8 => gl::RGBA,
                                gltf::image::Format::R16 => gl::RED,
                                gltf::image::Format::R16G16 => gl::RG,
                                gltf::image::Format::R16G16B16 => gl::RGB,
                                gltf::image::Format::R16G16B16A16 => gl::RGBA,
                                gltf::image::Format::R32G32B32FLOAT => gl::RGB,
                                gltf::image::Format::R32G32B32A32FLOAT => gl::RGBA,
                            },
                            match i.format {
                                gltf::image::Format::R8
                                | gltf::image::Format::R8G8
                                | gltf::image::Format::R8G8B8
                                | gltf::image::Format::R8G8B8A8 => gl::UNSIGNED_BYTE,
                                gltf::image::Format::R16
                                | gltf::image::Format::R16G16
                                | gltf::image::Format::R16G16B16
                                | gltf::image::Format::R16G16B16A16 => gl::UNSIGNED_SHORT,
                                _ => gl::FLOAT,
                            },
                            i.pixels.as_ptr() as *const std::ffi::c_void,
                        ),
                    };
                unsafe {
                    let mut texture: gl::types::GLuint = 0;
                    gl::GenTextures(1, &mut texture as *mut gl::types::GLuint);
                    gl::BindTexture(gl::TEXTURE_2D, texture);
                    gl::TexImage2D(
                        gl::TEXTURE_2D,
                        0,
                        gl::SRGB8_ALPHA8 as i32,
                        tw as i32,
                        th as i32,
                        0,
                        fmt,
                        ty,
                        ptr,
                    );
                    gl::GenerateMipmap(gl::TEXTURE_2D);
                    texture
                }
            })
            .collect();

        let meshes = gltf
            .meshes()
            .map(|m| {
                let primitives = m
                    .primitives()
                    .filter_map(|p| {
                        log::debug!("begin primitive");
                        let mode = match p.mode() {
                            gltf::mesh::Mode::Points => gl::POINTS,
                            gltf::mesh::Mode::Lines => gl::LINES,
                            gltf::mesh::Mode::LineLoop => gl::LINE_LOOP,
                            gltf::mesh::Mode::LineStrip => gl::LINE_STRIP,
                            gltf::mesh::Mode::Triangles => gl::TRIANGLES,
                            gltf::mesh::Mode::TriangleStrip => gl::TRIANGLE_STRIP,
                            gltf::mesh::Mode::TriangleFan => gl::TRIANGLE_FAN,
                        };
                        unsafe {
                            let mut vao: gl::types::GLuint = 0;
                            gl::GenVertexArrays(1, &mut vao as *mut gl::types::GLuint);
                            gl::BindVertexArray(vao);

                            let indices_accessor = p.indices()?;
                            let indices_view = indices_accessor.view()?;
                            let indices_buf = bufs.get(indices_view.buffer().index())?.0;
                            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, indices_buf);

                            // 这个图元上成功挂上的表情顶点属性槽（仅用于日志）
                            let mut bound: Vec<gl::types::GLuint> = Vec::new();
                            for (semantic, accessor) in p.attributes() {
                                let mattrib = match semantic {
                                    gltf::Semantic::Positions => Some(utils::ATTRIB_VERTEX),
                                    gltf::Semantic::Normals => Some(utils::ATTRIB_NORMAL),
                                    gltf::Semantic::TexCoords(0) => Some(utils::ATTRIB_TEXCOORD),
                                    gltf::Semantic::Joints(0) => Some(utils::ATTRIB_JOINT),
                                    gltf::Semantic::Weights(0) => Some(utils::ATTRIB_WEIGHT),
                                    _ => None,
                                };
                                if let Some(attrib) = mattrib {
                                    Self::initialize_attrib(attrib, &accessor, &bufs);
                                }
                            }

                            for (expr, mesh, attrib) in expressions.values() {
                                if m.index() == mesh.index() {
                                    match p.morph_targets().nth(expr.index as _) {
                                        Some(target) => match target.positions() {
                                            Some(pos) => {
                                                Self::initialize_attrib(*attrib, &pos, &bufs);
                                                bound.push(*attrib);
                                            }
                                            None => log::debug!(
                                                "表情 {} 在 mesh {} 图元里的 target {} 没有 POSITION 偏移",
                                                expr.index, m.index(), expr.index
                                            ),
                                        },
                                        None => log::debug!(
                                            "mesh {} 的图元没有第 {} 个 morph target",
                                            m.index(),
                                            expr.index
                                        ),
                                    }
                                }
                            }
                            if !bound.is_empty() {
                                log::debug!(
                                    "mesh {} 图元: 绑定了 {} 个表情顶点属性 {:?}",
                                    m.index(),
                                    bound.len(),
                                    bound
                                );
                            }

                            // 取这个图元 POSITION 的 Y 范围，用于按高度剔除
                            let (min_y, max_y) = p
                                .get(&gltf::Semantic::Positions)
                                .and_then(|a| Some((a.min()?, a.max()?)))
                                .map(|(mn, mx)| {
                                    let y = |v: &gltf::json::Value| {
                                        v.get(1).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32
                                    };
                                    (y(&mn), y(&mx))
                                })
                                .unwrap_or((0.0, 0.0));
                            Some(Primitive {
                                vao,
                                max_y,
                                min_y,
                                mode,
                                count: indices_accessor.count() as _,
                                index_type: match indices_accessor.data_type() {
                                    gltf::accessor::DataType::I8 => gl::BYTE,
                                    gltf::accessor::DataType::U8 => gl::UNSIGNED_BYTE,
                                    gltf::accessor::DataType::I16 => gl::SHORT,
                                    gltf::accessor::DataType::U16 => gl::UNSIGNED_SHORT,
                                    gltf::accessor::DataType::U32 => gl::UNSIGNED_INT,
                                    gltf::accessor::DataType::F32 => gl::FLOAT,
                                },
                                index_offset: indices_view.offset() as _,
                                material_index: p.material().index().unwrap(),
                            })
                        }
                    })
                    .collect();
                Mesh { primitives }
            })
            .collect();

        let skins = gltf
            .skins()
            .map(|s| {
                let get_buffer_data =
                    |buffer: gltf::Buffer| bufs.get(buffer.index()).map(|x| &*x.1 .0);
                Skin {
                    inverse_bind_matrices: s
                        .reader(get_buffer_data)
                        .read_inverse_bind_matrices()
                        .unwrap()
                        .map(|m| glam::Mat4::from_cols_array_2d(&m))
                        .collect(),
                    joints: s.joints().map(|j| j.index()).collect(),
                }
            })
            .collect();

        let materials = gltf
            .materials()
            .map(|m| {
                let name_upper = m.name().unwrap_or("").to_uppercase();
                let part = if name_upper.contains("HAIR") {
                    2
                } else if name_upper.contains("EYE") {
                    3
                } else if name_upper.contains("CLOTH") || name_upper.contains("OUTERWEAR") {
                    4
                } else if name_upper.contains("SKIN") || name_upper.contains("FACE") {
                    1
                } else {
                    9
                };
                let pbr = m.pbr_metallic_roughness();
                let [bcr, bcg, bcb, bca] = pbr.base_color_factor();
                let [emx, emy, emz] = m.emissive_factor();
                Material {
                    part,
                    base_color_factor: glam::Vec4::new(bcr, bcg, bcb, bca),
                    base_color_texture: pbr.base_color_texture().and_then(|t| {
                        let sampler = t.texture().sampler();
                        let tid = *tids.get(t.texture().source().index())?;
                        unsafe {
                            gl::BindTexture(gl::TEXTURE_2D, tid);
                            gl::TexParameteri(
                                gl::TEXTURE_2D,
                                gl::TEXTURE_WRAP_S,
                                sampler.wrap_s().as_gl_enum() as i32,
                            );
                            gl::TexParameteri(
                                gl::TEXTURE_2D,
                                gl::TEXTURE_WRAP_T,
                                sampler.wrap_t().as_gl_enum() as i32,
                            );
                            if let Some(min_filter) = sampler.min_filter() {
                                gl::TexParameteri(
                                    gl::TEXTURE_2D,
                                    gl::TEXTURE_MIN_FILTER,
                                    min_filter.as_gl_enum() as i32,
                                );
                            }
                            if let Some(mag_filter) = sampler.mag_filter() {
                                gl::TexParameteri(
                                    gl::TEXTURE_2D,
                                    gl::TEXTURE_MAG_FILTER,
                                    mag_filter.as_gl_enum() as i32,
                                );
                            }
                        }
                        Some(Texture { tid })
                    }),
                    metallic_factor: pbr.metallic_factor(),
                    roughness_factor: pbr.roughness_factor(),
                    metallic_roughness_texture: None,
                    normal_texture: None,
                    occlusion_texture: None,
                    emissive_factor: glam::Vec3::new(emx, emy, emz),
                    emissive_texture: None,
                }
            })
            .collect();

        let nodes = gltf
            .nodes()
            .map(|n| {
                // 注意：这里以前是 `{:?}` 打印整棵 Skin（含整份 glTF 文档），
                // 每个蒙皮节点一条、单条 220 KB，一次启动就能写出 1.3 MB 日志。
                // 只保留有用信息：节点名 + 蒙皮索引。
                log::debug!(
                    "node {} {:?}: skin={:?}",
                    n.index(),
                    n.name(),
                    n.skin().map(|s| s.index())
                );
                Node {
                    child_indices: n.children().map(|c| c.index()).collect(),
                    mesh_index: n.mesh().map(|m| m.index()),
                    skin_index: n.skin().map(|s| s.index()),
                    transform: glam::Mat4::from_cols_array_2d(&n.transform().matrix()),
                }
            })
            .collect();

        let bone_node_indices = vrm
            .humanoid
            .human_bones
            .iter()
            .map(|(nm, b)| (nm.clone(), b.node as _))
            .collect();

        let scene_node_indices = gltf
            .default_scene()
            .unwrap()
            .nodes()
            .map(|n| n.index())
            .collect();

        Self {
            meshes,
            skins,
            materials,
            nodes,
            bone_node_indices,
            scene_node_indices,
        }
    }

    fn compute_global_transforms_from(
        &self,
        transforms: &mut Vec<glam::Mat4>,
        nodes: &Vec<Node>,
        mat: &glam::Mat4,
        node_index: usize,
    ) {
        let node = &nodes[node_index];
        let newmat = mat.mul_mat4(&node.transform);
        transforms[node_index] = newmat;
        for ci in &node.child_indices {
            self.compute_global_transforms_from(transforms, nodes, &newmat, *ci);
        }
    }

    pub fn compute_global_transforms(
        &self,
        nodes: &Vec<Node>,
        mat: &glam::Mat4,
    ) -> Vec<glam::Mat4> {
        let mut global_transforms = vec![glam::Mat4::IDENTITY; self.nodes.len()];
        for ni in &self.scene_node_indices {
            self.compute_global_transforms_from(&mut global_transforms, nodes, mat, *ni);
        }
        global_transforms
    }

    pub fn render_node(
        &self,
        ctx: &context::Context,
        shader: &shader::Shader,
        global_transforms: &Vec<glam::Mat4>,
        node_index: usize,
    ) {
        let node = &self.nodes[node_index];
        let transform = &global_transforms[node_index];
        let max_joints = shader.max_joints.max(1) as usize;
        let mut joint_matrices = vec![glam::Mat4::IDENTITY; max_joints];
        if let Some(skin) = node.skin_index.and_then(|i| self.skins.get(i)) {
            if skin.joints.len() > max_joints {
                log::warn!("这套骨骼有 {} 根，超过 MAX_JOINTS={}，超出部分不会形变（可设 COLONQ_MAX_JOINTS 调大）",
                           skin.joints.len(), max_joints);
            }
            for (idx, ni) in skin.joints.iter().enumerate().take(max_joints) {
                let jointnode_transform = &global_transforms[*ni];
                let ibm = skin.inverse_bind_matrices[idx];
                joint_matrices[idx] = jointnode_transform.mul_mat4(&ibm);
            }
        }
        if let Some(m) = node.mesh_index.and_then(|i| self.meshes.get(i)) {
            let node_y = transform.w_axis.y;
            for p in &m.primitives {
                // 整个图元都在裁剪面以下 → 直接跳过，连顶点都不提交
                if shader.clip_y > -1.0e30 && p.max_y + node_y < shader.clip_y {
                    continue;
                }
                let base_color = self
                    .materials
                    .get(p.material_index)
                    .map(|m| m.base_color_factor)
                    .unwrap_or(glam::Vec4::ONE);
                if let Some(tex) = self
                    .materials
                    .get(p.material_index)
                    .and_then(|m| m.base_color_texture.as_ref())
                {
                    tex.bind(ctx);
                }
                let part_id = self
                    .materials
                    .get(p.material_index)
                    .map(|m| m.part)
                    .unwrap_or(9);
                unsafe {
                    gl::Uniform4fv(shader.uniform_base_color, 1, base_color.to_array().as_ptr());
                    gl::Uniform1f(shader.uniform_part_id, part_id as f32 / 255.0);
                    gl::UniformMatrix4fv(
                        shader.uniform_position,
                        1,
                        false as u8,
                        transform.to_cols_array().as_ptr(),
                    );
                    gl::UniformMatrix4fv(
                        shader.uniform_joint_matrices,
                        max_joints as i32,
                        false as u8,
                        joint_matrices
                            .iter()
                            .flat_map(|m| m.to_cols_array())
                            .collect::<Vec<f32>>()
                            .as_ptr(),
                    );

                    // 本体（描边壳由 Scene::render 统一在前一遍画完）
                    gl::BindVertexArray(p.vao);
                    gl::DrawElements(p.mode, p.count, p.index_type, p.index_offset as _);
                }
            }
        }
        for ci in &node.child_indices {
            self.render_node(ctx, shader, global_transforms, *ci);
        }
    }

    pub fn render(
        &self,
        ctx: &context::Context,
        shader: &shader::Shader,
        global_transforms: &Vec<glam::Mat4>,
    ) {
        // 描边要整场景两遍：先把所有节点的描边壳画完，再画所有本体。
        // 否则后画的节点（比如头发）的壳会盖住先画好的本体（比如脸）。
        if shader.outline_width > 0.0 {
            unsafe {
                gl::Uniform1i(shader.uniform_outline_pass, 1);
                gl::Enable(gl::CULL_FACE);
                gl::CullFace(gl::FRONT);
                gl::DepthMask(gl::FALSE); // 只测深度不写，避免壳挡住本体
            }
            for ni in &self.scene_node_indices {
                self.render_node(ctx, shader, global_transforms, *ni);
            }
            unsafe {
                gl::DepthMask(gl::TRUE);
                gl::Disable(gl::CULL_FACE);
                gl::Uniform1i(shader.uniform_outline_pass, 0);
            }
        }
        for ni in &self.scene_node_indices {
            self.render_node(ctx, shader, global_transforms, *ni);
        }

        // 部位编号 pass：只写 alpha 通道（ColorMask 关掉 RGB、关掉混合），
        // 这样主帧的 RGB 不受影响，alpha 里带着"这个像素属于哪个部位"
        if shader.parts_enabled {
            unsafe {
                gl::ColorMask(0, 0, 0, 1);
                gl::Disable(gl::BLEND);
                gl::Uniform1i(shader.uniform_part_pass, 1);
            }
            for ni in &self.scene_node_indices {
                self.render_node(ctx, shader, global_transforms, *ni);
            }
            unsafe {
                gl::ColorMask(1, 1, 1, 1);
                gl::Enable(gl::BLEND);
                gl::Uniform1i(shader.uniform_part_pass, 0);
            }
        }
    }
}
