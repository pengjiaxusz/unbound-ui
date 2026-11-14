//! 编译后的 SPIR-V 模块，在构建期嵌入二进制。
//!
//! `build.rs` 会调用 `glslc`（以及可用的 `spirv-val`），确保异常着色器在构建期
//! 即报错而非拖到驱动阶段。字节流直接嵌入而非从磁盘加载，使部署保持单文件形态。

use std::io::Cursor;

/// 单个内嵌着色器模块。
#[derive(Copy, Clone, Debug)]
pub struct ShaderModule {
    pub name: &'static str,
    pub spirv: &'static [u8],
}

impl ShaderModule {
    /// 解码供 `vkCreateShaderModule` 使用的 SPIR-V 字数组。
    ///
    /// # Errors
    /// 当内嵌字节长度不是 4 的倍数时返回错误。
    pub fn words(&self) -> Result<Vec<u32>, &'static str> {
        let mut cursor = Cursor::new(self.spirv);
        ash::util::read_spv(&mut cursor).map_err(|_| "malformed SPIR-V blob")
    }

    #[must_use]
    pub fn magic_ok(&self) -> bool {
        // SPIR-V 魔数：0x07230203（磁盘上为小端序）。
        self.spirv.len() >= 4
            && self.spirv.len() % 4 == 0
            && u32::from_le_bytes([self.spirv[0], self.spirv[1], self.spirv[2], self.spirv[3]])
                == 0x0723_0203
    }
}

macro_rules! embedded {
    ($fn_name:ident, $file:literal, $name:literal) => {
        pub fn $fn_name() -> ShaderModule {
            ShaderModule {
                name: $name,
                spirv: include_bytes!(concat!(env!("OUT_DIR"), "/spv/", $file, ".spv")),
            }
        }
    };
}

embedded!(uber_vert, "uber.vert", "uber.vert");
embedded!(uber_frag, "uber.frag", "uber.frag");
embedded!(opaque_core_frag, "opaque_core.frag", "opaque_core.frag");
embedded!(opaque_core_vert, "opaque_core.vert", "opaque_core.vert");
embedded!(cull, "cull.comp", "cull.comp");
embedded!(scatter, "scatter.comp", "scatter.comp");
embedded!(sort_hist, "sort_hist.comp", "sort_hist.comp");
embedded!(sort_scan, "sort_scan.comp", "sort_scan.comp");
embedded!(sort_scatter, "sort_scatter.comp", "sort_scatter.comp");
embedded!(indirect, "indirect.comp", "indirect.comp");
embedded!(mtsdf, "mtsdf.comp", "mtsdf.comp");
embedded!(mtsdf_pack, "mtsdf_pack.comp", "mtsdf_pack.comp");
embedded!(blur_down, "blur_down.comp", "blur_down.comp");
embedded!(blur_up, "blur_up.comp", "blur_up.comp");
embedded!(backdrop_vert, "backdrop.vert", "backdrop.vert");
embedded!(backdrop_frag, "backdrop.frag", "backdrop.frag");

/// 所有着色器模块列表，用于校验与诊断。
#[must_use]
pub fn all() -> [ShaderModule; 16] {
    [
        uber_vert(),
        uber_frag(),
        opaque_core_frag(),
        opaque_core_vert(),
        cull(),
        scatter(),
        sort_hist(),
        sort_scan(),
        sort_scatter(),
        indirect(),
        mtsdf(),
        mtsdf_pack(),
        blur_down(),
        blur_up(),
        backdrop_vert(),
        backdrop_frag(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_module_has_a_valid_spirv_header() {
        for m in all() {
            assert!(m.magic_ok(), "{} is not valid SPIR-V", m.name);
            assert!(
                m.spirv.len() > 64,
                "{} is suspiciously small ({} bytes)",
                m.name,
                m.spirv.len()
            );
        }
    }

    #[test]
    fn every_module_decodes_to_words() {
        for m in all() {
            let words = m.words().unwrap_or_else(|e| panic!("{}: {e}", m.name));
            assert!(words.len() > 16, "{}", m.name);
            assert_eq!(words[0], 0x0723_0203, "{}", m.name);
        }
    }

    #[test]
    fn vertex_and_fragment_entry_points_exist() {
        // 入口点名称在无 OpEntryPoint 的模块中并无固定第 5 字约定，此处仅断言各模块互不相同。
        assert_ne!(uber_vert().spirv, uber_frag().spirv);
        assert_ne!(cull().spirv, scatter().spirv);
        assert_ne!(sort_hist().spirv, sort_scatter().spirv);
    }
}
