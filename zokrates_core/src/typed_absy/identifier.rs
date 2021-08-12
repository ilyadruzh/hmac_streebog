use crate::typed_absy::types::FunctionKeyHash;
use crate::typed_absy::TypedModuleId;
use std::fmt;

#[derive(Debug, PartialEq, Clone, Hash, Eq)]
pub enum CoreIdentifier<'ast> {
    Source(&'ast str),
    Internal(&'static str, usize),
    Call(FunctionKeyHash, usize),
}

impl<'ast> fmt::Display for CoreIdentifier<'ast> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            CoreIdentifier::Source(s) => write!(f, "{}", s),
            CoreIdentifier::Internal(s, i) => write!(f, "#INTERNAL#_{}_{}", s, i),
            CoreIdentifier::Call(k, i) => write!(f, "{:x}_{}", k, i),
        }
    }
}

/// A identifier for a variable
#[derive(Debug, PartialEq, Clone, Hash, Eq)]
pub struct Identifier<'ast> {
    /// the id of the variable
    pub id: CoreIdentifier<'ast>,
    /// the version of the variable, used after SSA transformation
    pub version: usize,
    /// the call stack of the variable, used when inlining
    pub stack: Vec<(TypedModuleId, FunctionKeyHash, usize)>,
}

impl<'ast> fmt::Display for Identifier<'ast> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        if self.stack.len() == 0 && self.version == 0 {
            write!(f, "{}", self.id)
        } else {
            write!(
                f,
                "{}_{}_{}",
                self.stack
                    .iter()
                    .map(|(name, key_hash, count)| format!(
                        "{}_{}_{}",
                        name.display(),
                        key_hash,
                        count
                    ))
                    .collect::<Vec<_>>()
                    .join("_"),
                self.id,
                self.version
            )
        }
    }
}

impl<'ast> From<&'ast str> for Identifier<'ast> {
    fn from(id: &'ast str) -> Identifier<'ast> {
        Identifier::from(CoreIdentifier::Source(id))
    }
}

impl<'ast> From<CoreIdentifier<'ast>> for Identifier<'ast> {
    fn from(id: CoreIdentifier<'ast>) -> Identifier<'ast> {
        Identifier {
            id,
            version: 0,
            stack: vec![],
        }
    }
}

#[cfg(test)]
impl<'ast> Identifier<'ast> {
    pub fn version(mut self, version: usize) -> Self {
        self.version = version;
        self
    }

    pub fn stack(mut self, stack: Vec<(TypedModuleId, FunctionKeyHash, usize)>) -> Self {
        self.stack = stack;
        self
    }
}
