//! 渲染图（`docs` §10.1，M3）。
//!
//! 渲染图是对帧行为的声明：哪个 Pass 读取哪张纹理、哪个 Pass 写入哪张纹理。
//! 基于该声明，渲染图推导出 Pass 的*执行顺序*以及每张瞬态纹理的*生命周期*，
//! 从而使临时纹理在最后一个读取者完成后即可立即复用。
//!
//!
//!
//! 编译阶段用于暴露配置错误：在写入前被读取、未经屏障即被重复写入，或生命周期结束后
//! 仍被使用的纹理，都会在屏幕上表现为花屏，或在数帧后触发校验错误。渲染图会在编译期
//! 提前拦截此类问题。
//!

use std::collections::HashMap;

use crate::pool::TextureDesc;

/// 渲染图中的逻辑纹理句柄。
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureId(pub u32);

/// 纹理在帧中所扮演的角色。
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TextureRole {
    /// 在渲染图外部生成（例如交换链图像）。
    External,
    /// 在渲染图内部创建并销毁。
    Transient,
}

/// Pass 的操作类型。
#[derive(Clone, Debug, PartialEq)]
pub enum PassKind {
    /// 将交换链图像拷贝到临时纹理。
    CopyBackdrop,
    /// 按指定半径对纹理进行模糊。
    Blur {
        /// 模糊半径（像素）。
        radius: u32,
    },
    /// 将模糊纹理合成到背景之上。
    Composite,
    /// 将最终图像绘制到交换链。
    Present,
}

/// 单个渲染 Pass。
#[derive(Clone, Debug, PartialEq)]
pub struct Pass {
    /// Pass 名称，用于诊断信息。
    pub name: String,
    /// 该 Pass 读取的纹理列表。
    pub reads: Vec<TextureId>,
    /// 该 Pass 写入的纹理列表。
    pub writes: Vec<TextureId>,
    /// 该 Pass 的具体操作。
    pub kind: PassKind,
}

/// 纹理声明。
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    /// 纹理描述信息。
    pub desc: TextureDesc,
    /// 纹理角色。
    pub role: TextureRole,
}

/// 渲染图编译失败错误。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphError {
    /// 某 Pass 读取了没有任何其他 Pass 写入的纹理。
    ReadBeforeWrite {
        /// 出错的纹理。
        texture: TextureId,
        /// 读取该纹理的 Pass 名称。
        pass: String,
    },
    /// 同一纹理被两个 Pass 连续写入，期间未被消费。
    DoubleWrite {
        /// 出错的纹理。
        texture: TextureId,
        /// 第二个写入该纹理的 Pass 名称。
        pass: String,
    },
    /// Pass 之间存在循环依赖。
    Cycle,
    /// 某 Pass 引用了不存在的纹理。
    UnknownTexture {
        /// 出错的纹理。
        texture: TextureId,
        /// 引用该纹理的 Pass 名称。
        pass: String,
    },
    /// 渲染图为空。
    Empty,
}

impl core::fmt::Display for GraphError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ReadBeforeWrite { texture, pass } => write!(
                f,
                "pass {pass:?} reads texture {} before it is written",
                texture.0
            ),
            Self::DoubleWrite { texture, pass } => {
                write!(
                    f,
                    "texture {} is written twice, last by {pass:?}",
                    texture.0
                )
            }
            Self::Cycle => write!(f, "the passes form a cycle"),
            Self::UnknownTexture { texture, pass } => {
                write!(f, "pass {pass:?} refers to unknown texture {}", texture.0)
            }
            Self::Empty => write!(f, "the graph is empty"),
        }
    }
}

impl std::error::Error for GraphError {}

/// 纹理及其存活的 Pass 区间。
#[derive(Clone, Debug, PartialEq)]
pub struct Lifetime {
    /// 纹理句柄。
    pub texture: TextureId,
    /// 首次访问该纹理的 Pass 索引。
    pub first_use: usize,
    /// 最后访问该纹理的 Pass 索引。
    pub last_use: usize,
    /// 纹理角色。
    pub role: TextureRole,
}

/// 编译后的渲染图：包含 Pass 执行顺序及各纹理生命周期。
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledGraph {
    /// 按执行顺序排列的 Pass 列表。
    pub passes: Vec<Pass>,
    /// 每张纹理的生命周期。
    pub lifetimes: Vec<Lifetime>,
}

impl CompiledGraph {
    /// 访问指定纹理的 Pass 区间（起止索引）。
    #[must_use]
    pub fn passes_for(&self, texture: TextureId) -> Vec<usize> {
        self.passes
            .iter()
            .enumerate()
            .filter(|(_, pass)| pass.reads.contains(&texture) || pass.writes.contains(&texture))
            .map(|(index, _)| index)
            .collect()
    }

    /// 查询指定纹理的生命周期。
    #[must_use]
    pub fn lifetime(&self, texture: TextureId) -> Option<&Lifetime> {
        self.lifetimes.iter().find(|life| life.texture == texture)
    }

    /// 在指定 Pass 结束生命周期的纹理列表。
    ///
    /// 纹理池可在该 Pass 结束后立即重新分配这些纹理。
    #[must_use]
    pub fn expiring_after(&self, pass_index: usize) -> Vec<TextureId> {
        self.lifetimes
            .iter()
            .filter(|life| life.last_use == pass_index && life.role == TextureRole::Transient)
            .map(|life| life.texture)
            .collect()
    }
}

/// 构建中的渲染图。
#[derive(Clone, Debug, Default)]
pub struct RenderGraph {
    textures: Vec<Texture>,
    passes: Vec<Pass>,
}

impl RenderGraph {
    /// 创建空渲染图。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 声明一张纹理。
    pub fn add_texture(&mut self, desc: TextureDesc, role: TextureRole) -> TextureId {
        let id = TextureId(self.textures.len() as u32);
        self.textures.push(Texture { desc, role });
        id
    }

    /// 声明一个 Pass。
    pub fn add_pass(
        &mut self,
        name: impl Into<String>,
        kind: PassKind,
        reads: &[TextureId],
        writes: &[TextureId],
    ) {
        self.passes.push(Pass {
            name: name.into(),
            reads: reads.to_vec(),
            writes: writes.to_vec(),
            kind,
        });
    }

    /// 已声明的纹理数量。
    #[must_use]
    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    /// 已声明的 Pass 数量。
    #[must_use]
    pub fn pass_count(&self) -> usize {
        self.passes.len()
    }

    /// 获取纹理描述信息。
    #[must_use]
    pub fn texture_desc(&self, texture: TextureId) -> Option<TextureDesc> {
        self.textures
            .get(texture.0 as usize)
            .map(|entry| entry.desc)
    }

    /// 编译渲染图。
    ///
    /// # Errors
    /// 当渲染图存在不一致时返回 [`GraphError`]。
    pub fn compile(&self) -> Result<CompiledGraph, GraphError> {
        if self.passes.is_empty() {
            return Err(GraphError::Empty);
        }
        // 按数据依赖关系对 Pass 排序：每个 Pass 必须在其读取纹理的写入者之后运行。
        //
        let order = self.topological_order()?;
        let mut ordered: Vec<Pass> = order
            .iter()
            .map(|index| self.passes[*index].clone())
            .collect();

        // 校验读写配对并计算各纹理的生命周期。
        let mut written: HashMap<TextureId, String> = HashMap::new();
        let mut first_use: HashMap<TextureId, usize> = HashMap::new();
        let mut last_use: HashMap<TextureId, usize> = HashMap::new();

        for (index, pass) in ordered.iter().enumerate() {
            for texture in &pass.reads {
                let Some(entry) = self.textures.get(texture.0 as usize) else {
                    return Err(GraphError::UnknownTexture {
                        texture: *texture,
                        pass: pass.name.clone(),
                    });
                };
                // 外部纹理在渲染图之外生成（由呈现管线提供），因此读取它总是合法的。
                // 瞬态纹理则必须已被先前的 Pass 写入。
                //
                if entry.role == TextureRole::Transient && !written.contains_key(texture) {
                    return Err(GraphError::ReadBeforeWrite {
                        texture: *texture,
                        pass: pass.name.clone(),
                    });
                }
                first_use.entry(*texture).or_insert(index);
                last_use.insert(*texture, index);
            }
            for texture in &pass.writes {
                let Some(entry) = self.textures.get(texture.0 as usize) else {
                    return Err(GraphError::UnknownTexture {
                        texture: *texture,
                        pass: pass.name.clone(),
                    });
                };
                // 每张瞬态纹理只能有一个写入者；外部纹理归呈现管线所有，不在此处限制。
                //
                if entry.role == TextureRole::Transient && written.contains_key(texture) {
                    return Err(GraphError::DoubleWrite {
                        texture: *texture,
                        pass: pass.name.clone(),
                    });
                }
                written.insert(*texture, pass.name.clone());
                first_use.entry(*texture).or_insert(index);
                last_use.insert(*texture, index);
            }
        }

        let mut lifetimes: Vec<Lifetime> = first_use
            .into_iter()
            .map(|(texture, first)| Lifetime {
                texture,
                first_use: first,
                last_use: last_use.get(&texture).copied().unwrap_or(first),
                role: self.textures[texture.0 as usize].role,
            })
            .collect();
        lifetimes.sort_by_key(|life| life.texture);
        ordered.shrink_to_fit();
        Ok(CompiledGraph {
            passes: ordered,
            lifetimes,
        })
    }

    /// 根据依赖关系对 Pass 进行拓扑排序。
    fn topological_order(&self) -> Result<Vec<usize>, GraphError> {
        // Pass 依赖于写入其所读取纹理的所有 Pass。
        let mut writer: HashMap<TextureId, usize> = HashMap::new();
        for (index, pass) in self.passes.iter().enumerate() {
            for texture in &pass.writes {
                // 仅瞬态纹理会产生依赖：外部纹理由呈现管线生成，而非由 Pass 生成。
                //
                if self
                    .textures
                    .get(texture.0 as usize)
                    .is_some_and(|entry| entry.role == TextureRole::Transient)
                {
                    writer.insert(*texture, index);
                }
            }
        }
        let mut visited = vec![0u8; self.passes.len()];
        let mut order = Vec::with_capacity(self.passes.len());
        // 深度优先后序遍历：确保 Pass 在其所有依赖项之后输出。
        //
        fn visit(
            index: usize,
            passes: &[Pass],
            writer: &HashMap<TextureId, usize>,
            visited: &mut Vec<u8>,
            order: &mut Vec<usize>,
        ) -> Result<(), GraphError> {
            match visited[index] {
                2 => return Ok(()),
                1 => return Err(GraphError::Cycle),
                _ => {}
            }
            visited[index] = 1;
            for texture in &passes[index].reads {
                if let Some(&producer) = writer.get(texture) {
                    if producer != index {
                        visit(producer, passes, writer, visited, order)?;
                    }
                }
            }
            visited[index] = 2;
            order.push(index);
            Ok(())
        }
        for index in 0..self.passes.len() {
            visit(index, &self.passes, &writer, &mut visited, &mut order)?;
        }
        Ok(order)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pool::TextureFormat;

    fn desc(width: u32, height: u32) -> TextureDesc {
        TextureDesc::new(width, height, TextureFormat::Rgba16f)
    }

    /// 毛玻璃渲染链：拷贝背景、执行模糊、进行合成。
    fn frosted_graph() -> RenderGraph {
        let mut graph = RenderGraph::new();
        // 交换链图像归呈现管线所有。
        let swapchain = graph.add_texture(desc(1920, 1080), TextureRole::External);
        let scratch = graph.add_texture(desc(960, 540), TextureRole::Transient);
        let blurred = graph.add_texture(desc(960, 540), TextureRole::Transient);
        let composed = graph.add_texture(desc(1920, 1080), TextureRole::External);
        graph.add_pass("copy", PassKind::CopyBackdrop, &[swapchain], &[scratch]);
        graph.add_pass(
            "blur",
            PassKind::Blur { radius: 24 },
            &[scratch],
            &[blurred],
        );
        graph.add_pass("composite", PassKind::Composite, &[blurred], &[composed]);
        graph.add_pass("present", PassKind::Present, &[composed], &[swapchain]);
        graph
    }

    #[test]
    fn a_frosted_graph_compiles_into_a_sane_order() {
        let graph = frosted_graph();
        let compiled = graph.compile().unwrap();
        let names: Vec<&str> = compiled
            .passes
            .iter()
            .map(|pass| pass.name.as_str())
            .collect();
        assert_eq!(names, vec!["copy", "blur", "composite", "present"]);
    }

    #[test]
    fn a_transient_texture_is_released_after_its_last_reader() {
        let graph = frosted_graph();
        let compiled = graph.compile().unwrap();
        let scratch = TextureId(1);
        let blurred = TextureId(2);
        // 临时纹理在模糊 Pass 后结束生命周期，模糊纹理在合成 Pass 后结束生命周期。
        //
        let expiring = compiled.expiring_after(1);
        assert!(expiring.contains(&scratch));
        let expiring = compiled.expiring_after(2);
        assert!(expiring.contains(&blurred));
    }

    #[test]
    fn reading_a_texture_before_it_is_written_is_rejected() {
        let mut graph = RenderGraph::new();
        let a = graph.add_texture(desc(64, 64), TextureRole::Transient);
        let b = graph.add_texture(desc(64, 64), TextureRole::Transient);
        graph.add_pass("read", PassKind::Blur { radius: 4 }, &[a], &[b]);
        let error = graph.compile().unwrap_err();
        assert!(matches!(error, GraphError::ReadBeforeWrite { .. }));
    }

    #[test]
    fn writing_a_texture_twice_is_rejected() {
        let mut graph = RenderGraph::new();
        let a = graph.add_texture(desc(64, 64), TextureRole::Transient);
        graph.add_pass("first", PassKind::Blur { radius: 4 }, &[], &[a]);
        graph.add_pass("second", PassKind::Blur { radius: 4 }, &[], &[a]);
        let error = graph.compile().unwrap_err();
        assert!(matches!(error, GraphError::DoubleWrite { .. }));
    }

    #[test]
    fn an_unknown_texture_is_rejected() {
        let mut graph = RenderGraph::new();
        graph.add_pass("ghost", PassKind::Blur { radius: 4 }, &[TextureId(99)], &[]);
        let error = graph.compile().unwrap_err();
        assert!(matches!(error, GraphError::UnknownTexture { .. }));
    }

    #[test]
    fn an_empty_graph_is_rejected() {
        let graph = RenderGraph::new();
        assert_eq!(graph.compile().unwrap_err(), GraphError::Empty);
    }

    #[test]
    fn a_cycle_is_rejected() {
        let mut graph = RenderGraph::new();
        let a = graph.add_texture(desc(64, 64), TextureRole::Transient);
        let b = graph.add_texture(desc(64, 64), TextureRole::Transient);
        graph.add_pass("a", PassKind::Blur { radius: 4 }, &[b], &[a]);
        graph.add_pass("b", PassKind::Blur { radius: 4 }, &[a], &[b]);
        assert_eq!(graph.compile().unwrap_err(), GraphError::Cycle);
    }

    #[test]
    fn the_pass_order_follows_the_data_dependencies() {
        let mut graph = RenderGraph::new();
        let a = graph.add_texture(desc(64, 64), TextureRole::Transient);
        let b = graph.add_texture(desc(64, 64), TextureRole::Transient);
        // 故意按错误顺序声明 Pass，以测试拓扑排序。
        graph.add_pass("second", PassKind::Blur { radius: 4 }, &[a], &[b]);
        graph.add_pass("first", PassKind::Blur { radius: 4 }, &[], &[a]);
        let compiled = graph.compile().unwrap();
        let names: Vec<&str> = compiled
            .passes
            .iter()
            .map(|pass| pass.name.as_str())
            .collect();
        assert_eq!(names, vec!["first", "second"]);
    }
}
