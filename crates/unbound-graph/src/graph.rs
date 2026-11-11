//! The render graph (`docs` §10.1, M3).
//!
//! A render graph is a declaration of what the frame does: which pass reads
//! which texture and which pass writes it. From that declaration the graph
//! derives the *order* of the passes and the *lifetime* of every transient
//! texture, so a scratch texture can be reused as soon as its last reader is
//! done.
//!
//! The compile step is where the mistakes surface. A texture that is read
//! before it is written, written twice without a barrier, or used after its
//! lifetime ended is a bug that shows up as garbage on screen — or as a
//! validation error three frames later. The graph rejects it up front.

use std::collections::HashMap;

use crate::pool::TextureDesc;

/// A logical texture inside the graph.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextureId(pub u32);

/// The role a texture plays in the frame.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum TextureRole {
    /// Produced outside the graph (the swapchain image, for instance).
    External,
    /// Created and destroyed inside the graph.
    Transient,
}

/// What a pass does.
#[derive(Clone, Debug, PartialEq)]
pub enum PassKind {
    /// Copies the swapchain image into a scratch texture.
    CopyBackdrop,
    /// Blurs a texture with the given radius.
    Blur {
        /// The blur radius in pixels.
        radius: u32,
    },
    /// Composites a blurred texture over the backdrop.
    Composite,
    /// Draws the final image into the swapchain.
    Present,
}

/// A single pass.
#[derive(Clone, Debug, PartialEq)]
pub struct Pass {
    /// The name of the pass, for diagnostics.
    pub name: String,
    /// The textures the pass reads.
    pub reads: Vec<TextureId>,
    /// The textures the pass writes.
    pub writes: Vec<TextureId>,
    /// What the pass does.
    pub kind: PassKind,
}

/// A texture declaration.
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    /// The description of the texture.
    pub desc: TextureDesc,
    /// The role of the texture.
    pub role: TextureRole,
}

/// The graph failed to compile.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphError {
    /// A pass reads a texture no other pass writes.
    ReadBeforeWrite {
        /// The offending texture.
        texture: TextureId,
        /// The pass that read it.
        pass: String,
    },
    /// A texture is written by two passes without being consumed in between.
    DoubleWrite {
        /// The offending texture.
        texture: TextureId,
        /// The second writer.
        pass: String,
    },
    /// The passes form a cycle.
    Cycle,
    /// A pass refers to a texture that does not exist.
    UnknownTexture {
        /// The offending texture.
        texture: TextureId,
        /// The pass that referred to it.
        pass: String,
    },
    /// The graph is empty.
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

/// A texture together with the pass range it is live for.
#[derive(Clone, Debug, PartialEq)]
pub struct Lifetime {
    /// The texture.
    pub texture: TextureId,
    /// The first pass that touches it.
    pub first_use: usize,
    /// The last pass that touches it.
    pub last_use: usize,
    /// The role of the texture.
    pub role: TextureRole,
}

/// A compiled graph: the pass order plus the lifetimes.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledGraph {
    /// The passes, in execution order.
    pub passes: Vec<Pass>,
    /// The lifetime of every texture.
    pub lifetimes: Vec<Lifetime>,
}

impl CompiledGraph {
    /// The passes that touch a texture.
    #[must_use]
    pub fn passes_for(&self, texture: TextureId) -> Vec<usize> {
        self.passes
            .iter()
            .enumerate()
            .filter(|(_, pass)| pass.reads.contains(&texture) || pass.writes.contains(&texture))
            .map(|(index, _)| index)
            .collect()
    }

    /// The lifetime of a texture.
    #[must_use]
    pub fn lifetime(&self, texture: TextureId) -> Option<&Lifetime> {
        self.lifetimes.iter().find(|life| life.texture == texture)
    }

    /// The textures whose lifetime ends at the given pass.
    ///
    /// These are the ones the pool can hand out again right after that pass.
    #[must_use]
    pub fn expiring_after(&self, pass_index: usize) -> Vec<TextureId> {
        self.lifetimes
            .iter()
            .filter(|life| life.last_use == pass_index && life.role == TextureRole::Transient)
            .map(|life| life.texture)
            .collect()
    }
}

/// The render graph under construction.
#[derive(Clone, Debug, Default)]
pub struct RenderGraph {
    textures: Vec<Texture>,
    passes: Vec<Pass>,
}

impl RenderGraph {
    /// Create an empty graph.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Declare a texture.
    pub fn add_texture(&mut self, desc: TextureDesc, role: TextureRole) -> TextureId {
        let id = TextureId(self.textures.len() as u32);
        self.textures.push(Texture { desc, role });
        id
    }

    /// Declare a pass.
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

    /// The number of declared textures.
    #[must_use]
    pub fn texture_count(&self) -> usize {
        self.textures.len()
    }

    /// The number of declared passes.
    #[must_use]
    pub fn pass_count(&self) -> usize {
        self.passes.len()
    }

    /// The description of a texture.
    #[must_use]
    pub fn texture_desc(&self, texture: TextureId) -> Option<TextureDesc> {
        self.textures
            .get(texture.0 as usize)
            .map(|entry| entry.desc)
    }

    /// Compile the graph.
    ///
    /// # Errors
    /// Returns a [`GraphError`] when the graph is inconsistent.
    pub fn compile(&self) -> Result<CompiledGraph, GraphError> {
        if self.passes.is_empty() {
            return Err(GraphError::Empty);
        }
        // Order the passes by their data dependencies: a pass runs after every
        // pass that writes something it reads.
        let order = self.topological_order()?;
        let mut ordered: Vec<Pass> = order
            .iter()
            .map(|index| self.passes[*index].clone())
            .collect();

        // Validate the write/read pairs and compute the lifetimes.
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
                // An external texture is produced outside the graph (by the
                // presentation path), so reading it is always legal. A
                // transient one must have been written already.
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
                // A transient texture has exactly one producer; an external
                // one belongs to the presentation path and is not policed.
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

    /// Order the passes by their dependencies.
    fn topological_order(&self) -> Result<Vec<usize>, GraphError> {
        // A pass depends on the pass that writes each texture it reads.
        let mut writer: HashMap<TextureId, usize> = HashMap::new();
        for (index, pass) in self.passes.iter().enumerate() {
            for texture in &pass.writes {
                // Only transient textures create a dependency: an external
                // texture is produced by the presentation path, not by a pass.
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
        // Depth-first post-order: a pass is emitted after everything it
        // depends on.
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

    /// The frosted-glass chain: copy the backdrop, blur it, composite it.
    fn frosted_graph() -> RenderGraph {
        let mut graph = RenderGraph::new();
        // The swapchain image is owned by the presentation path.
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
        // The scratch texture dies after the blur pass, the blurred one after
        // the composite pass.
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
        // The passes are declared in the wrong order on purpose.
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
