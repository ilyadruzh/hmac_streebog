pub mod folder;
mod from_typed;
mod identifier;
mod parameter;
pub mod types;
mod uint;
mod variable;

pub use self::parameter::Parameter;
pub use self::types::Type;
pub use self::variable::Variable;
pub use crate::zir::uint::{ShouldReduce, UExpression, UExpressionInner, UMetadata};
use std::path::PathBuf;

use crate::embed::FlatEmbed;
use crate::zir::types::{FunctionKey, Signature};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use zokrates_field::Field;

pub use self::folder::Folder;

pub use self::identifier::{Identifier, SourceIdentifier};

/// An identifier for a `ZirModule`. Typically a path or uri.
pub type ZirModuleId = PathBuf;

/// A collection of `ZirModule`s
pub type ZirModules<'ast, T> = HashMap<ZirModuleId, ZirModule<'ast, T>>;

/// A collection of `ZirFunctionSymbol`s
/// # Remarks
/// * It is the role of the semantic checker to make sure there are no duplicates for a given `FunctionKey`
///   in a given `ZirModule`, hence the use of a HashMap
pub type ZirFunctionSymbols<'ast, T> = HashMap<FunctionKey<'ast>, ZirFunctionSymbol<'ast, T>>;

/// A typed program as a collection of modules, one of them being the main
#[derive(PartialEq, Debug)]
pub struct ZirProgram<'ast, T> {
    pub modules: ZirModules<'ast, T>,
    pub main: ZirModuleId,
}

impl<'ast, T: fmt::Display> fmt::Display for ZirProgram<'ast, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for (module_id, module) in &self.modules {
            writeln!(
                f,
                "| {}: |{}",
                module_id.display(),
                if *module_id == self.main {
                    "<---- main"
                } else {
                    ""
                }
            )?;
            writeln!(f, "{}", "-".repeat(100))?;
            writeln!(f, "{}", module)?;
            writeln!(f, "{}", "-".repeat(100))?;
            writeln!(f, "")?;
        }
        write!(f, "")
    }
}

/// A typed program as a collection of functions. Types have been resolved during semantic checking.
#[derive(PartialEq, Clone)]
pub struct ZirModule<'ast, T> {
    /// Functions of the program
    pub functions: ZirFunctionSymbols<'ast, T>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ZirFunctionSymbol<'ast, T> {
    Here(ZirFunction<'ast, T>),
    There(FunctionKey<'ast>, ZirModuleId),
    Flat(FlatEmbed),
}

impl<'ast, T> ZirFunctionSymbol<'ast, T> {
    pub fn signature<'a>(&'a self, modules: &'a ZirModules<T>) -> Signature {
        match self {
            ZirFunctionSymbol::Here(f) => f.signature.clone(),
            ZirFunctionSymbol::There(key, module_id) => modules
                .get(module_id)
                .unwrap()
                .functions
                .get(key)
                .unwrap()
                .signature(&modules)
                .clone(),
            ZirFunctionSymbol::Flat(flat_fun) => flat_fun.signature().into(),
        }
    }
}

impl<'ast, T: fmt::Display> fmt::Display for ZirModule<'ast, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let res = self
            .functions
            .iter()
            .map(|(key, symbol)| match symbol {
                ZirFunctionSymbol::Here(ref function) => format!("def {}{}", key.id, function),
                ZirFunctionSymbol::There(ref fun_key, ref module_id) => format!(
                    "import {} from \"{}\" as {} // with signature {}",
                    fun_key.id,
                    module_id.display(),
                    key.id,
                    key.signature
                ),
                ZirFunctionSymbol::Flat(ref flat_fun) => {
                    format!("def {}{}:\n\t// hidden", key.id, flat_fun.signature())
                }
            })
            .collect::<Vec<_>>();
        write!(f, "{}", res.join("\n"))
    }
}

impl<'ast, T: fmt::Debug> fmt::Debug for ZirModule<'ast, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "module(\n\tfunctions:\n\t\t{}\n)",
            self.functions
                .iter()
                .map(|x| format!("{:?}", x))
                .collect::<Vec<_>>()
                .join("\n\t\t")
        )
    }
}

/// A typed function
#[derive(Clone, PartialEq)]
pub struct ZirFunction<'ast, T> {
    /// Arguments of the function
    pub arguments: Vec<Parameter<'ast>>,
    /// Vector of statements that are executed when running the function
    pub statements: Vec<ZirStatement<'ast, T>>,
    /// function signature
    pub signature: Signature,
}

impl<'ast, T: fmt::Display> fmt::Display for ZirFunction<'ast, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "({}) -> ({}):\n{}",
            self.arguments
                .iter()
                .map(|x| format!("{}", x))
                .collect::<Vec<_>>()
                .join(", "),
            self.signature
                .outputs
                .iter()
                .map(|x| format!("{}", x))
                .collect::<Vec<_>>()
                .join(", "),
            self.statements
                .iter()
                .map(|x| format!("\t{}", x))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

impl<'ast, T: fmt::Debug> fmt::Debug for ZirFunction<'ast, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "ZirFunction(arguments: {:?}, ...):\n{}",
            self.arguments,
            self.statements
                .iter()
                .map(|x| format!("\t{:?}", x))
                .collect::<Vec<_>>()
                .join("\n")
        )
    }
}

pub type ZirAssignee<'ast> = Variable<'ast>;

/// A statement in a `ZirFunction`
#[derive(Clone, PartialEq, Hash, Eq)]
pub enum ZirStatement<'ast, T> {
    Return(Vec<ZirExpression<'ast, T>>),
    Definition(ZirAssignee<'ast>, ZirExpression<'ast, T>),
    Declaration(Variable<'ast>),
    Assertion(BooleanExpression<'ast, T>),
    MultipleDefinition(Vec<ZirAssignee<'ast>>, ZirExpressionList<'ast, T>),
}

impl<'ast, T: fmt::Debug> fmt::Debug for ZirStatement<'ast, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ZirStatement::Return(ref exprs) => {
                write!(f, "Return(")?;
                for (i, expr) in exprs.iter().enumerate() {
                    write!(f, "{:?}", expr)?;
                    if i < exprs.len() - 1 {
                        write!(f, ", ")?;
                    }
                }
                write!(f, ")")
            }
            ZirStatement::Declaration(ref var) => write!(f, "Declaration({:?})", var),
            ZirStatement::Definition(ref lhs, ref rhs) => {
                write!(f, "Definition({:?}, {:?})", lhs, rhs)
            }
            ZirStatement::Assertion(ref e) => write!(f, "Assertion({:?})", e),
            ZirStatement::MultipleDefinition(ref lhs, ref rhs) => {
                write!(f, "MultipleDefinition({:?}, {:?})", lhs, rhs)
            }
        }
    }
}

impl<'ast, T: fmt::Display> fmt::Display for ZirStatement<'ast, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ZirStatement::Return(ref exprs) => {
                write!(f, "return ")?;
                for (i, expr) in exprs.iter().enumerate() {
                    write!(f, "{}", expr)?;
                    if i < exprs.len() - 1 {
                        write!(f, ", ")?;
                    }
                }
                write!(f, "")
            }
            ZirStatement::Declaration(ref var) => write!(f, "assert({})", var),
            ZirStatement::Definition(ref lhs, ref rhs) => write!(f, "{} = {}", lhs, rhs),
            ZirStatement::Assertion(ref e) => write!(f, "{}", e),
            ZirStatement::MultipleDefinition(ref ids, ref rhs) => {
                for (i, id) in ids.iter().enumerate() {
                    write!(f, "{}", id)?;
                    if i < ids.len() - 1 {
                        write!(f, ", ")?;
                    }
                }
                write!(f, " = {}", rhs)
            }
        }
    }
}

pub trait Typed {
    fn get_type(&self) -> Type;
}

/// A typed expression
#[derive(Clone, PartialEq, Hash, Eq)]
pub enum ZirExpression<'ast, T> {
    Boolean(BooleanExpression<'ast, T>),
    FieldElement(FieldElementExpression<'ast, T>),
    Uint(UExpression<'ast, T>),
}

impl<'ast, T: Field> From<BooleanExpression<'ast, T>> for ZirExpression<'ast, T> {
    fn from(e: BooleanExpression<'ast, T>) -> ZirExpression<T> {
        ZirExpression::Boolean(e)
    }
}

impl<'ast, T: Field> From<FieldElementExpression<'ast, T>> for ZirExpression<'ast, T> {
    fn from(e: FieldElementExpression<'ast, T>) -> ZirExpression<T> {
        ZirExpression::FieldElement(e)
    }
}

impl<'ast, T: Field> From<UExpression<'ast, T>> for ZirExpression<'ast, T> {
    fn from(e: UExpression<'ast, T>) -> ZirExpression<T> {
        ZirExpression::Uint(e)
    }
}

impl<'ast, T: fmt::Display> fmt::Display for ZirExpression<'ast, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ZirExpression::Boolean(ref e) => write!(f, "{}", e),
            ZirExpression::FieldElement(ref e) => write!(f, "{}", e),
            ZirExpression::Uint(ref e) => write!(f, "{}", e),
        }
    }
}

impl<'ast, T: fmt::Debug> fmt::Debug for ZirExpression<'ast, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ZirExpression::Boolean(ref e) => write!(f, "{:?}", e),
            ZirExpression::FieldElement(ref e) => write!(f, "{:?}", e),
            ZirExpression::Uint(ref e) => write!(f, "{:?}", e),
        }
    }
}

impl<'ast, T: Field> Typed for ZirExpression<'ast, T> {
    fn get_type(&self) -> Type {
        match *self {
            ZirExpression::Boolean(ref e) => e.get_type(),
            ZirExpression::FieldElement(ref e) => e.get_type(),
            ZirExpression::Uint(ref e) => e.get_type(),
        }
    }
}

impl<'ast, T: Field> Typed for FieldElementExpression<'ast, T> {
    fn get_type(&self) -> Type {
        Type::FieldElement
    }
}

impl<'ast, T: Field> Typed for UExpression<'ast, T> {
    fn get_type(&self) -> Type {
        Type::Uint(self.bitwidth)
    }
}

impl<'ast, T: Field> Typed for BooleanExpression<'ast, T> {
    fn get_type(&self) -> Type {
        Type::Boolean
    }
}

pub trait MultiTyped {
    fn get_types(&self) -> &Vec<Type>;
}

#[derive(Clone, PartialEq, Hash, Eq)]
pub enum ZirExpressionList<'ast, T> {
    FunctionCall(FunctionKey<'ast>, Vec<ZirExpression<'ast, T>>, Vec<Type>),
}

impl<'ast, T: Field> MultiTyped for ZirExpressionList<'ast, T> {
    fn get_types(&self) -> &Vec<Type> {
        match *self {
            ZirExpressionList::FunctionCall(_, _, ref types) => types,
        }
    }
}

/// An expression of type `field`
#[derive(Clone, PartialEq, Hash, Eq)]
pub enum FieldElementExpression<'ast, T> {
    Number(T),
    Identifier(Identifier<'ast>),
    Add(
        Box<FieldElementExpression<'ast, T>>,
        Box<FieldElementExpression<'ast, T>>,
    ),
    Sub(
        Box<FieldElementExpression<'ast, T>>,
        Box<FieldElementExpression<'ast, T>>,
    ),
    Mult(
        Box<FieldElementExpression<'ast, T>>,
        Box<FieldElementExpression<'ast, T>>,
    ),
    Div(
        Box<FieldElementExpression<'ast, T>>,
        Box<FieldElementExpression<'ast, T>>,
    ),
    Pow(
        Box<FieldElementExpression<'ast, T>>,
        Box<FieldElementExpression<'ast, T>>,
    ),
    IfElse(
        Box<BooleanExpression<'ast, T>>,
        Box<FieldElementExpression<'ast, T>>,
        Box<FieldElementExpression<'ast, T>>,
    ),
}

/// An expression of type `bool`
#[derive(Clone, PartialEq, Hash, Eq)]
pub enum BooleanExpression<'ast, T> {
    Identifier(Identifier<'ast>),
    Value(bool),
    Lt(
        Box<FieldElementExpression<'ast, T>>,
        Box<FieldElementExpression<'ast, T>>,
    ),
    Le(
        Box<FieldElementExpression<'ast, T>>,
        Box<FieldElementExpression<'ast, T>>,
    ),
    FieldEq(
        Box<FieldElementExpression<'ast, T>>,
        Box<FieldElementExpression<'ast, T>>,
    ),
    BoolEq(
        Box<BooleanExpression<'ast, T>>,
        Box<BooleanExpression<'ast, T>>,
    ),
    UintEq(Box<UExpression<'ast, T>>, Box<UExpression<'ast, T>>),
    Ge(
        Box<FieldElementExpression<'ast, T>>,
        Box<FieldElementExpression<'ast, T>>,
    ),
    Gt(
        Box<FieldElementExpression<'ast, T>>,
        Box<FieldElementExpression<'ast, T>>,
    ),
    Or(
        Box<BooleanExpression<'ast, T>>,
        Box<BooleanExpression<'ast, T>>,
    ),
    And(
        Box<BooleanExpression<'ast, T>>,
        Box<BooleanExpression<'ast, T>>,
    ),
    Not(Box<BooleanExpression<'ast, T>>),
    IfElse(
        Box<BooleanExpression<'ast, T>>,
        Box<BooleanExpression<'ast, T>>,
        Box<BooleanExpression<'ast, T>>,
    ),
}

pub struct ConjunctionIterator<T> {
    current: Vec<T>,
}

impl<'ast, T> Iterator for ConjunctionIterator<BooleanExpression<'ast, T>> {
    type Item = BooleanExpression<'ast, T>;

    fn next(&mut self) -> Option<Self::Item> {
        self.current
            .pop()
            .map(|n| match n {
                BooleanExpression::And(box left, box right) => {
                    self.current.push(left);
                    self.current.push(right);
                    self.next()
                }
                n => Some(n),
            })
            .flatten()
    }
}

impl<'ast, T> BooleanExpression<'ast, T> {
    pub fn into_conjunction_iterator(self) -> ConjunctionIterator<Self> {
        ConjunctionIterator {
            current: vec![self],
        }
    }
}

// Downcasts
impl<'ast, T> TryFrom<ZirExpression<'ast, T>> for FieldElementExpression<'ast, T> {
    type Error = ();

    fn try_from(
        te: ZirExpression<'ast, T>,
    ) -> Result<FieldElementExpression<'ast, T>, Self::Error> {
        match te {
            ZirExpression::FieldElement(e) => Ok(e),
            _ => Err(()),
        }
    }
}

impl<'ast, T> TryFrom<ZirExpression<'ast, T>> for BooleanExpression<'ast, T> {
    type Error = ();

    fn try_from(te: ZirExpression<'ast, T>) -> Result<BooleanExpression<'ast, T>, Self::Error> {
        match te {
            ZirExpression::Boolean(e) => Ok(e),
            _ => Err(()),
        }
    }
}

impl<'ast, T> TryFrom<ZirExpression<'ast, T>> for UExpression<'ast, T> {
    type Error = ();

    fn try_from(te: ZirExpression<'ast, T>) -> Result<UExpression<'ast, T>, Self::Error> {
        match te {
            ZirExpression::Uint(e) => Ok(e),
            _ => Err(()),
        }
    }
}

impl<'ast, T: fmt::Display> fmt::Display for FieldElementExpression<'ast, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            FieldElementExpression::Number(ref i) => write!(f, "{}", i),
            FieldElementExpression::Identifier(ref var) => write!(f, "{}", var),
            FieldElementExpression::Add(ref lhs, ref rhs) => write!(f, "({} + {})", lhs, rhs),
            FieldElementExpression::Sub(ref lhs, ref rhs) => write!(f, "({} - {})", lhs, rhs),
            FieldElementExpression::Mult(ref lhs, ref rhs) => write!(f, "({} * {})", lhs, rhs),
            FieldElementExpression::Div(ref lhs, ref rhs) => write!(f, "({} / {})", lhs, rhs),
            FieldElementExpression::Pow(ref lhs, ref rhs) => write!(f, "{}**{}", lhs, rhs),
            FieldElementExpression::IfElse(ref condition, ref consequent, ref alternative) => {
                write!(
                    f,
                    "if {} then {} else {} fi",
                    condition, consequent, alternative
                )
            }
        }
    }
}

impl<'ast, T: fmt::Display> fmt::Display for UExpression<'ast, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.inner {
            UExpressionInner::Value(ref v) => write!(f, "{}", v),
            UExpressionInner::Identifier(ref var) => write!(f, "{}", var),
            UExpressionInner::Add(ref lhs, ref rhs) => write!(f, "({} + {})", lhs, rhs),
            UExpressionInner::Sub(ref lhs, ref rhs) => write!(f, "({} - {})", lhs, rhs),
            UExpressionInner::Mult(ref lhs, ref rhs) => write!(f, "({} * {})", lhs, rhs),
            UExpressionInner::Div(ref lhs, ref rhs) => write!(f, "({} * {})", lhs, rhs),
            UExpressionInner::Rem(ref lhs, ref rhs) => write!(f, "({} % {})", lhs, rhs),
            UExpressionInner::Xor(ref lhs, ref rhs) => write!(f, "({} ^ {})", lhs, rhs),
            UExpressionInner::And(ref lhs, ref rhs) => write!(f, "({} & {})", lhs, rhs),
            UExpressionInner::Or(ref lhs, ref rhs) => write!(f, "({} | {})", lhs, rhs),
            UExpressionInner::LeftShift(ref e, ref by) => write!(f, "({} << {})", e, by),
            UExpressionInner::RightShift(ref e, ref by) => write!(f, "({} >> {})", e, by),
            UExpressionInner::Not(ref e) => write!(f, "!{}", e),
            UExpressionInner::IfElse(ref condition, ref consequent, ref alternative) => write!(
                f,
                "if {} then {} else {} fi",
                condition, consequent, alternative
            ),
        }
    }
}

impl<'ast, T: fmt::Display> fmt::Display for BooleanExpression<'ast, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            BooleanExpression::Identifier(ref var) => write!(f, "{}", var),
            BooleanExpression::Lt(ref lhs, ref rhs) => write!(f, "{} < {}", lhs, rhs),
            BooleanExpression::Le(ref lhs, ref rhs) => write!(f, "{} <= {}", lhs, rhs),
            BooleanExpression::FieldEq(ref lhs, ref rhs) => write!(f, "{} == {}", lhs, rhs),
            BooleanExpression::BoolEq(ref lhs, ref rhs) => write!(f, "{} == {}", lhs, rhs),
            BooleanExpression::UintEq(ref lhs, ref rhs) => write!(f, "{} == {}", lhs, rhs),
            BooleanExpression::Ge(ref lhs, ref rhs) => write!(f, "{} >= {}", lhs, rhs),
            BooleanExpression::Gt(ref lhs, ref rhs) => write!(f, "{} > {}", lhs, rhs),
            BooleanExpression::Or(ref lhs, ref rhs) => write!(f, "{} || {}", lhs, rhs),
            BooleanExpression::And(ref lhs, ref rhs) => write!(f, "{} && {}", lhs, rhs),
            BooleanExpression::Not(ref exp) => write!(f, "!{}", exp),
            BooleanExpression::Value(b) => write!(f, "{}", b),
            BooleanExpression::IfElse(ref condition, ref consequent, ref alternative) => write!(
                f,
                "if {} then {} else {} fi",
                condition, consequent, alternative
            ),
        }
    }
}

impl<'ast, T: fmt::Debug> fmt::Debug for BooleanExpression<'ast, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

impl<'ast, T: fmt::Debug> fmt::Debug for FieldElementExpression<'ast, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            FieldElementExpression::Number(ref i) => write!(f, "Num({:?})", i),
            FieldElementExpression::Identifier(ref var) => write!(f, "Ide({:?})", var),
            FieldElementExpression::Add(ref lhs, ref rhs) => write!(f, "Add({:?}, {:?})", lhs, rhs),
            FieldElementExpression::Sub(ref lhs, ref rhs) => write!(f, "Sub({:?}, {:?})", lhs, rhs),
            FieldElementExpression::Mult(ref lhs, ref rhs) => {
                write!(f, "Mult({:?}, {:?})", lhs, rhs)
            }
            FieldElementExpression::Div(ref lhs, ref rhs) => write!(f, "Div({:?}, {:?})", lhs, rhs),
            FieldElementExpression::Pow(ref lhs, ref rhs) => write!(f, "Pow({:?}, {:?})", lhs, rhs),
            FieldElementExpression::IfElse(ref condition, ref consequent, ref alternative) => {
                write!(
                    f,
                    "IfElse({:?}, {:?}, {:?})",
                    condition, consequent, alternative
                )
            }
        }
    }
}

impl<'ast, T: fmt::Display> fmt::Display for ZirExpressionList<'ast, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ZirExpressionList::FunctionCall(ref key, ref p, _) => {
                write!(f, "{}(", key.id,)?;
                for (i, param) in p.iter().enumerate() {
                    write!(f, "{}", param)?;
                    if i < p.len() - 1 {
                        write!(f, ", ")?;
                    }
                }
                write!(f, ")")
            }
        }
    }
}

impl<'ast, T: fmt::Debug> fmt::Debug for ZirExpressionList<'ast, T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ZirExpressionList::FunctionCall(ref i, ref p, _) => {
                write!(f, "FunctionCall({:?}, (", i)?;
                f.debug_list().entries(p.iter()).finish()?;
                write!(f, ")")
            }
        }
    }
}

// Common behaviour accross expressions

pub trait IfElse<'ast, T> {
    fn if_else(condition: BooleanExpression<'ast, T>, consequence: Self, alternative: Self)
        -> Self;
}

impl<'ast, T> IfElse<'ast, T> for FieldElementExpression<'ast, T> {
    fn if_else(
        condition: BooleanExpression<'ast, T>,
        consequence: Self,
        alternative: Self,
    ) -> Self {
        FieldElementExpression::IfElse(box condition, box consequence, box alternative)
    }
}

impl<'ast, T> IfElse<'ast, T> for BooleanExpression<'ast, T> {
    fn if_else(
        condition: BooleanExpression<'ast, T>,
        consequence: Self,
        alternative: Self,
    ) -> Self {
        BooleanExpression::IfElse(box condition, box consequence, box alternative)
    }
}

impl<'ast, T> IfElse<'ast, T> for UExpression<'ast, T> {
    fn if_else(
        condition: BooleanExpression<'ast, T>,
        consequence: Self,
        alternative: Self,
    ) -> Self {
        let bitwidth = consequence.bitwidth;

        UExpressionInner::IfElse(box condition, box consequence, box alternative).annotate(bitwidth)
    }
}
