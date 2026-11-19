//! lang-allow: 中文注释与文档
//! 表单状态绑定系统：支持声明期绑定与运行时事件突变双向同步。

use rustc_hash::FxHashMap;
use std::any::Any;

/// 表单控件传递的值枚举。
#[derive(Debug, Clone, PartialEq)]
pub enum WidgetValue {
    Text(String),
    Float(f32),
    Bool(bool),
}

impl WidgetValue {
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s.as_str()),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_float(&self) -> Option<f32> {
        match self {
            Self::Float(v) => Some(*v),
            _ => None,
        }
    }

    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

/// 类型擦除的状态突变器闭包。
pub type FormMutator = Box<dyn FnMut(&mut dyn Any, &WidgetValue)>;

/// 表单双向绑定存储：记录节点键（`u64`）到对应状态突变器的映射。
#[derive(Default)]
pub struct FormBindingStore {
    bindings: FxHashMap<u64, FormMutator>,
}

impl FormBindingStore {
    pub fn new() -> Self {
        Self {
            bindings: FxHashMap::default(),
        }
    }

    /// 清空所有注册的绑定。
    pub fn clear(&mut self) {
        self.bindings.clear();
    }

    /// 检查指定键是否注册了表单绑定。
    #[must_use]
    pub fn contains(&self, key: u64) -> bool {
        self.bindings.contains_key(&key)
    }

    /// 注册一个通用的表单突变器。
    pub fn insert(&mut self, key: u64, mutator: FormMutator) {
        self.bindings.insert(key, mutator);
    }

    /// 注册文本框输入绑定。
    pub fn insert_text<S: 'static>(
        &mut self,
        key: u64,
        mut setter: impl FnMut(&mut S, &str) + 'static,
    ) {
        self.insert(
            key,
            Box::new(move |any_state: &mut dyn Any, val: &WidgetValue| {
                if let Some(state) = any_state.downcast_mut::<S>() {
                    if let Some(text) = val.as_text() {
                        setter(state, text);
                    }
                }
            }),
        );
    }

    /// 注册滑块浮点值绑定。
    pub fn insert_slider<S: 'static>(
        &mut self,
        key: u64,
        mut setter: impl FnMut(&mut S, f32) + 'static,
    ) {
        self.insert(
            key,
            Box::new(move |any_state: &mut dyn Any, val: &WidgetValue| {
                if let Some(state) = any_state.downcast_mut::<S>() {
                    if let Some(v) = val.as_float() {
                        setter(state, v);
                    }
                }
            }),
        );
    }

    /// 注册开关布尔值绑定。
    pub fn insert_switch<S: 'static>(
        &mut self,
        key: u64,
        mut setter: impl FnMut(&mut S, bool) + 'static,
    ) {
        self.insert(
            key,
            Box::new(move |any_state: &mut dyn Any, val: &WidgetValue| {
                if let Some(state) = any_state.downcast_mut::<S>() {
                    if let Some(b) = val.as_bool() {
                        setter(state, b);
                    }
                }
            }),
        );
    }

    /// 应用指定键的绑定，将新值突变写回应用状态。
    pub fn apply(&mut self, key: u64, state: &mut dyn Any, val: &WidgetValue) -> bool {
        if let Some(mutator) = self.bindings.get_mut(&key) {
            mutator(state, val);
            true
        } else {
            false
        }
    }
}

impl core::fmt::Debug for FormBindingStore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("FormBindingStore")
            .field("count", &self.bindings.len())
            .finish()
    }
}
