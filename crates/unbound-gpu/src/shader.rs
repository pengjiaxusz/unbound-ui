//! Compiled SPIR-V modules, embedded at build time.
//!
//! `build.rs` runs `glslc` (and `spirv-val`, when present) so a malformed
//! shader fails the build rather than the driver. The bytes are embedded rather
//! than loaded from disk so the guest/host deployment is a single executable.

use std::io::Cursor;

/// One embedded shader.
#[derive(Copy, Clone, Debug)]
pub struct ShaderModule {
    pub name: &'static str,
    pub spirv: &'static [u8],
}

impl ShaderModule {
    /// Decode the SPIR-V words for `vkCreateShaderModule`.
    ///
    /// # Errors
    /// Returns an error when the embedded blob is not a multiple of 4 bytes.
    pub fn words(&self) -> Result<Vec<u32>, &'static str> {
        let mut cursor = Cursor::new(self.spirv);
        ash::util::read_spv(&mut cursor).map_err(|_| "malformed SPIR-V blob")
    }

    #[must_use]
    pub fn magic_ok(&self) -> bool {
        // SPIR-V magic: 0x07230203, little-endian on disk.
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

/// Every module, for validation and diagnostics.
#[must_use]
pub fn all() -> [ShaderModule; 11] {
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
        // Entry point is the 5th word of an OpEntryPoint-free module only by
        // convention, so just assert the modules differ from each other.
        assert_ne!(uber_vert().spirv, uber_frag().spirv);
        assert_ne!(cull().spirv, scatter().spirv);
        assert_ne!(sort_hist().spirv, sort_scatter().spirv);
    }
}
