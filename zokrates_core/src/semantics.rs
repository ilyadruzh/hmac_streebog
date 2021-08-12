//! Module containing semantic analysis tools to run at compile time
//!
//! @file semantics.rs
//! @author Thibaut Schaeffer <thibaut@schaeff.fr>
//! @date 2017

use crate::absy::Identifier;
use crate::absy::*;
use crate::typed_absy::*;
use crate::typed_absy::{Parameter, Variable};
use std::collections::{hash_map::Entry, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use zokrates_field::Field;

use crate::parser::Position;

use crate::absy::types::{UnresolvedSignature, UnresolvedType, UserTypeId};
use crate::typed_absy::types::{FunctionKey, Signature, StructLocation, Type};

use crate::typed_absy::types::{ArrayType, StructMember};
use std::hash::{Hash, Hasher};

#[derive(PartialEq, Debug)]
pub struct ErrorInner {
    pos: Option<(Position, Position)>,
    message: String,
}

#[derive(PartialEq, Debug)]
pub struct Error {
    pub inner: ErrorInner,
    pub module_id: PathBuf,
}

impl ErrorInner {
    fn in_file(self, id: &ModuleId) -> Error {
        Error {
            inner: self,
            module_id: id.clone(),
        }
    }
}

type TypeMap = HashMap<ModuleId, HashMap<UserTypeId, Type>>;

/// The global state of the program during semantic checks
#[derive(Debug)]
struct State<'ast, T: Field> {
    /// The modules yet to be checked, which we consume as we explore the dependency tree
    modules: Modules<'ast>,
    /// The already checked modules, which we're returning at the end
    typed_modules: TypedModules<'ast, T>,
    /// The user-defined types, which we keep track at this phase only. In later phases, we rely only on basic types and combinations thereof
    types: TypeMap,
}

/// A symbol for a given name: either a type or a group of functions. Not both!
#[derive(PartialEq, Hash, Eq, Debug)]
enum SymbolType {
    Type,
    Functions(BTreeSet<Signature>),
}

/// A data structure to keep track of all symbols in a module
#[derive(Default)]
struct SymbolUnifier {
    symbols: HashMap<String, SymbolType>,
}

impl SymbolUnifier {
    fn insert_type<S: Into<String>>(&mut self, id: S) -> bool {
        let s_type = self.symbols.entry(id.into());
        match s_type {
            // if anything is already called `id`, we cannot introduce this type
            Entry::Occupied(..) => false,
            // otherwise, we can!
            Entry::Vacant(v) => {
                v.insert(SymbolType::Type);
                true
            }
        }
    }

    fn insert_function<S: Into<String>>(&mut self, id: S, signature: Signature) -> bool {
        let s_type = self.symbols.entry(id.into());
        match s_type {
            // if anything is already called `id`, it depends what it is
            Entry::Occupied(mut o) => {
                match o.get_mut() {
                    // if it's a Type, then we can't introduce a function
                    SymbolType::Type => false,
                    // if it's a Function, we can introduce a new function only if it has a different signature
                    SymbolType::Functions(signatures) => signatures.insert(signature),
                }
            }
            // otherwise, we can!
            Entry::Vacant(v) => {
                v.insert(SymbolType::Functions(vec![signature].into_iter().collect()));
                true
            }
        }
    }
}

impl<'ast, T: Field> State<'ast, T> {
    fn new(modules: Modules<'ast>) -> Self {
        State {
            modules,
            typed_modules: HashMap::new(),
            types: HashMap::new(),
        }
    }
}

impl fmt::Display for ErrorInner {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let location = self
            .pos
            .map(|p| format!("{}", p.0))
            .unwrap_or("?".to_string());
        write!(f, "{}\n\t{}", location, self.message)
    }
}

/// A function query in the current module.
#[derive(Debug)]
struct FunctionQuery<'ast> {
    id: Identifier<'ast>,
    inputs: Vec<Type>,
    /// Output types are optional as we try to infer them
    outputs: Vec<Option<Type>>,
}

impl<'ast> fmt::Display for FunctionQuery<'ast> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "(")?;
        for (i, t) in self.inputs.iter().enumerate() {
            write!(f, "{}", t)?;
            if i < self.inputs.len() - 1 {
                write!(f, ", ")?;
            }
        }
        write!(f, ")")?;

        match self.outputs.len() {
            0 => write!(f, ""),
            1 => write!(
                f,
                " -> {}",
                match &self.outputs[0] {
                    Some(t) => format!("{}", t),
                    None => format!("_"),
                }
            ),
            _ => {
                write!(f, " -> (")?;
                for (i, t) in self.outputs.iter().enumerate() {
                    match t {
                        Some(t) => write!(f, "{}", t)?,
                        None => write!(f, "_")?,
                    }
                    if i < self.outputs.len() - 1 {
                        write!(f, ", ")?;
                    }
                }
                write!(f, ")")
            }
        }
    }
}

impl<'ast> FunctionQuery<'ast> {
    /// Create a new query.
    fn new(
        id: Identifier<'ast>,
        inputs: &Vec<Type>,
        outputs: &Vec<Option<Type>>,
    ) -> FunctionQuery<'ast> {
        FunctionQuery {
            id,
            inputs: inputs.clone(),
            outputs: outputs.clone(),
        }
    }

    /// match a `FunctionKey` against this `FunctionQuery`
    fn match_func(&self, func: &FunctionKey) -> bool {
        self.id == func.id
            && self.inputs == func.signature.inputs
            && self.outputs.len() == func.signature.outputs.len()
            && self.outputs.iter().enumerate().all(|(index, t)| match t {
                Some(ref t) => t == &func.signature.outputs[index],
                _ => true,
            })
    }

    fn match_funcs(&self, funcs: &HashSet<FunctionKey<'ast>>) -> Option<FunctionKey<'ast>> {
        funcs.iter().find(|func| self.match_func(func)).cloned()
    }
}

/// A scoped variable, so that we can delete all variables of a given scope when exiting it
#[derive(Clone, Debug)]
pub struct ScopedVariable<'ast> {
    id: Variable<'ast>,
    level: usize,
}

/// Identifiers of different `ScopedVariable`s should not conflict, so we define them as equivalent
impl<'ast> PartialEq for ScopedVariable<'ast> {
    fn eq(&self, other: &ScopedVariable) -> bool {
        self.id.id == other.id.id
    }
}

impl<'ast> Hash for ScopedVariable<'ast> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.id.id.hash(state);
    }
}

impl<'ast> Eq for ScopedVariable<'ast> {}

/// Checker checks the semantics of a program, keeping track of functions and variables in scope
pub struct Checker<'ast> {
    scope: HashSet<ScopedVariable<'ast>>,
    functions: HashSet<FunctionKey<'ast>>,
    level: usize,
}

impl<'ast> Checker<'ast> {
    fn new() -> Checker<'ast> {
        Checker {
            scope: HashSet::new(),
            functions: HashSet::new(),
            level: 0,
        }
    }

    /// Check a `Program`
    ///
    /// # Arguments
    ///
    /// * `prog` - The `Program` to be checked
    pub fn check<T: Field>(prog: Program<'ast>) -> Result<TypedProgram<'ast, T>, Vec<Error>> {
        Checker::new().check_program(prog)
    }

    fn check_program<T: Field>(
        &mut self,
        program: Program<'ast>,
    ) -> Result<TypedProgram<'ast, T>, Vec<Error>> {
        let mut state = State::new(program.modules);

        let mut errors = vec![];

        // recursively type-check modules starting with `main`
        match self.check_module(&program.main, &mut state) {
            Ok(()) => {}
            Err(e) => errors.extend(e),
        };

        if errors.len() > 0 {
            return Err(errors);
        }

        let main_id = program.main.clone();

        Checker::check_single_main(state.typed_modules.get(&program.main).unwrap()).map_err(
            |inner| {
                vec![Error {
                    inner,
                    module_id: main_id,
                }]
            },
        )?;

        Ok(TypedProgram {
            main: program.main,
            modules: state.typed_modules,
        })
    }

    fn check_struct_type_declaration(
        &mut self,
        id: String,
        s: StructDefinitionNode<'ast>,
        module_id: &ModuleId,
        types: &TypeMap,
    ) -> Result<Type, Vec<ErrorInner>> {
        let pos = s.pos();
        let s = s.value;

        let mut errors = vec![];
        let mut fields: Vec<(_, _)> = vec![];
        let mut fields_set = HashSet::new();

        for field in s.fields {
            let member_id = field.value.id.to_string();
            match self
                .check_type(field.value.ty, module_id, &types)
                .map(|t| (member_id, t))
            {
                Ok(f) => match fields_set.insert(f.0.clone()) {
                    true => fields.push(f),
                    false => errors.push(ErrorInner {
                        pos: Some(pos),
                        message: format!("Duplicate key {} in struct definition", f.0,),
                    }),
                },
                Err(e) => {
                    errors.push(e);
                }
            }
        }

        if errors.len() > 0 {
            return Err(errors);
        }

        Ok(Type::Struct(StructType::new(
            module_id.into(),
            id,
            fields
                .iter()
                .map(|f| StructMember::new(f.0.clone(), f.1.clone()))
                .collect(),
        )))
    }

    fn check_symbol_declaration<T: Field>(
        &mut self,
        declaration: SymbolDeclarationNode<'ast>,
        module_id: &ModuleId,
        state: &mut State<'ast, T>,
        functions: &mut HashMap<FunctionKey<'ast>, TypedFunctionSymbol<'ast, T>>,
        symbol_unifier: &mut SymbolUnifier,
    ) -> Result<(), Vec<Error>> {
        let mut errors: Vec<Error> = vec![];

        let pos = declaration.pos();
        let declaration = declaration.value;

        match declaration.symbol.clone() {
            Symbol::HereType(t) => {
                match self.check_struct_type_declaration(
                    declaration.id.to_string(),
                    t.clone(),
                    module_id,
                    &state.types,
                ) {
                    Ok(ty) => {
                        match symbol_unifier.insert_type(declaration.id) {
                            false => errors.push(
                                ErrorInner {
                                    pos: Some(pos),
                                    message: format!(
                                        "{} conflicts with another symbol",
                                        declaration.id,
                                    ),
                                }
                                .in_file(module_id),
                            ),
                            true => {}
                        };
                        state
                            .types
                            .entry(module_id.clone())
                            .or_default()
                            .insert(declaration.id.to_string(), ty);
                    }
                    Err(e) => errors.extend(e.into_iter().map(|inner| Error {
                        inner,
                        module_id: module_id.clone(),
                    })),
                }
            }
            Symbol::HereFunction(f) => match self.check_function(f, module_id, &state.types) {
                Ok(funct) => {
                    match symbol_unifier.insert_function(declaration.id, funct.signature.clone()) {
                        false => errors.push(
                            ErrorInner {
                                pos: Some(pos),
                                message: format!(
                                    "{} conflicts with another symbol",
                                    declaration.id,
                                ),
                            }
                            .in_file(module_id),
                        ),
                        true => {}
                    };

                    self.functions.insert(
                        FunctionKey::with_id(declaration.id.clone())
                            .signature(funct.signature.clone()),
                    );
                    functions.insert(
                        FunctionKey::with_id(declaration.id.clone())
                            .signature(funct.signature.clone()),
                        TypedFunctionSymbol::Here(funct),
                    );
                }
                Err(e) => {
                    errors.extend(e.into_iter().map(|inner| inner.in_file(module_id)));
                }
            },
            Symbol::There(import) => {
                let pos = import.pos();
                let import = import.value;

                match Checker::new().check_module(&import.module_id, state) {
                    Ok(()) => {
                        // find candidates in the checked module
                        let function_candidates: Vec<_> = state
                            .typed_modules
                            .get(&import.module_id)
                            .unwrap()
                            .functions
                            .iter()
                            .filter(|(k, _)| k.id == import.symbol_id)
                            .map(|(_, v)| FunctionKey {
                                id: import.symbol_id.clone(),
                                signature: v.signature(&state.typed_modules).clone(),
                            })
                            .collect();

                        // find candidates in the types
                        let type_candidate = state
                            .types
                            .entry(import.module_id.clone())
                            .or_default()
                            .get(import.symbol_id)
                            .cloned();

                        match (function_candidates.len(), type_candidate) {
                            (0, Some(t)) => {

                                // rename the type to the declared symbol
                                let t = match t {
                                    Type::Struct(t) => Type::Struct(StructType {
                                        location: Some(StructLocation {
                                            name: declaration.id.into(),
                                            module: module_id.clone()
                                        }),
                                        ..t
                                    }),
                                    _ => unreachable!()
                                };

                                // we imported a type, so the symbol it gets bound to should not already exist
                                match symbol_unifier.insert_type(declaration.id) {
                                    false => {
                                        errors.push(Error {
                                            module_id: module_id.clone(),
                                            inner: ErrorInner {
                                            pos: Some(pos),
                                            message: format!(
                                                "{} conflicts with another symbol",
                                                declaration.id,
                                            ),
                                        }});
                                    }
                                    true => {}
                                };
                                state
                                    .types
                                    .entry(module_id.clone())
                                    .or_default()
                                    .insert(declaration.id.to_string(), t.clone());
                            }
                            (0, None) => {
                                errors.push(ErrorInner {
                                    pos: Some(pos),
                                    message: format!(
                                        "Could not find symbol {} in module {}",
                                        import.symbol_id, import.module_id.display(),
                                    ),
                                }.in_file(module_id));
                            }
                            (_, Some(_)) => unreachable!("collision in module we're importing from should have been caught when checking it"),
                            _ => {
                                for candidate in function_candidates {

                                    match symbol_unifier.insert_function(declaration.id, candidate.signature.clone()) {
                                        false => {
                                            errors.push(ErrorInner {
                                                pos: Some(pos),
                                                message: format!(
                                                    "{} conflicts with another symbol",
                                                    declaration.id,
                                                ),
                                            }.in_file(module_id));
                                        },
                                        true => {}
                                    };

                                    self.functions.insert(candidate.clone().id(declaration.id));
                                    functions.insert(
                                        candidate.clone().id(declaration.id),
                                        TypedFunctionSymbol::There(
                                            candidate,
                                            import.module_id.clone(),
                                        ),
                                    );
                                }
                            }
                        };
                    }
                    Err(e) => {
                        errors.extend(e);
                    }
                };
            }
            Symbol::Flat(funct) => {
                match symbol_unifier.insert_function(declaration.id, funct.signature()) {
                    false => {
                        errors.push(
                            ErrorInner {
                                pos: Some(pos),
                                message: format!(
                                    "{} conflicts with another symbol",
                                    declaration.id,
                                ),
                            }
                            .in_file(module_id),
                        );
                    }
                    true => {}
                };

                self.functions.insert(
                    FunctionKey::with_id(declaration.id.clone())
                        .signature(funct.signature().clone()),
                );
                functions.insert(
                    FunctionKey::with_id(declaration.id.clone())
                        .signature(funct.signature().clone()),
                    TypedFunctionSymbol::Flat(funct),
                );
            }
        };

        // return if any errors occured
        if errors.len() > 0 {
            return Err(errors);
        }

        Ok(())
    }

    fn check_module<T: Field>(
        &mut self,
        module_id: &ModuleId,
        state: &mut State<'ast, T>,
    ) -> Result<(), Vec<Error>> {
        let mut errors = vec![];
        let mut checked_functions = HashMap::new();

        // check if the module was already removed from the untyped ones
        let to_insert = match state.modules.remove(module_id) {
            // if it was, do nothing
            None => None,
            // if it was not, check it
            Some(module) => {
                assert_eq!(module.imports.len(), 0);

                // we need to create an entry in the types map to store types for this module
                state.types.entry(module_id.clone()).or_default();

                // we keep track of the introduced symbols to avoid colisions between types and functions
                let mut symbol_unifier = SymbolUnifier::default();

                // we go through symbol declarations and check them
                for declaration in module.symbols {
                    match self.check_symbol_declaration(
                        declaration,
                        module_id,
                        state,
                        &mut checked_functions,
                        &mut symbol_unifier,
                    ) {
                        Ok(()) => {}
                        Err(e) => {
                            errors.extend(e);
                        }
                    }
                }

                Some(TypedModule {
                    functions: checked_functions,
                })
            }
        };

        // return if any errors occured
        if errors.len() > 0 {
            return Err(errors);
        }

        // insert into typed_modules if we checked anything
        match to_insert {
            Some(typed_module) => {
                // there should be no checked module at that key just yet, if there is we have a colision or we checked something twice
                assert!(state
                    .typed_modules
                    .insert(module_id.clone(), typed_module)
                    .is_none());
            }
            None => {}
        };

        Ok(())
    }

    fn check_single_main<T: Field>(module: &TypedModule<T>) -> Result<(), ErrorInner> {
        match module
            .functions
            .iter()
            .filter(|(key, _)| key.id == "main")
            .count()
        {
            1 => Ok(()),
            0 => Err(ErrorInner {
                pos: None,
                message: format!("No main function found"),
            }),
            n => Err(ErrorInner {
                pos: None,
                message: format!("Only one main function allowed, found {}", n),
            }),
        }
    }

    fn check_for_var(&self, var: &VariableNode) -> Result<(), ErrorInner> {
        match var.value.get_type() {
            UnresolvedType::FieldElement => Ok(()),
            t => Err(ErrorInner {
                pos: Some(var.pos()),
                message: format!("Variable in for loop cannot have type {}", t),
            }),
        }
    }

    fn check_function<T: Field>(
        &mut self,
        funct_node: FunctionNode<'ast>,
        module_id: &ModuleId,
        types: &TypeMap,
    ) -> Result<TypedFunction<'ast, T>, Vec<ErrorInner>> {
        self.enter_scope();

        let pos = funct_node.pos();

        let mut errors = vec![];
        let funct = funct_node.value;
        let mut arguments_checked = vec![];
        let mut signature = None;

        assert_eq!(funct.arguments.len(), funct.signature.inputs.len());

        for arg in funct.arguments {
            match self.check_parameter(arg, module_id, types) {
                Ok(a) => {
                    self.insert_into_scope(a.id.clone());
                    arguments_checked.push(a);
                }
                Err(e) => errors.extend(e),
            }
        }

        let mut statements_checked = vec![];

        match self.check_signature(funct.signature, module_id, types) {
            Ok(s) => {
                let mut found_return = false;

                for stat in funct.statements.into_iter() {
                    let pos = Some(stat.pos());

                    if let Statement::Return(..) = stat.value {
                        if found_return {
                            errors.push(ErrorInner {
                                pos,
                                message: format!("Expected a single return statement",),
                            });
                        }

                        found_return = true;
                    }

                    match self.check_statement(stat, module_id, types) {
                        Ok(statement) => {
                            match &statement {
                                TypedStatement::Return(e) => {
                                    match e.iter().map(|e| e.get_type()).collect::<Vec<_>>()
                                        == s.outputs
                                    {
                                        true => {}
                                        false => errors.push(ErrorInner {
                                            pos,
                                            message: format!(
                                                "Expected ({}) in return statement, found ({})",
                                                s.outputs
                                                    .iter()
                                                    .map(|t| t.to_string())
                                                    .collect::<Vec<_>>()
                                                    .join(", "),
                                                e.iter()
                                                    .map(|e| e.get_type())
                                                    .map(|t| t.to_string())
                                                    .collect::<Vec<_>>()
                                                    .join(", ")
                                            ),
                                        }),
                                    }
                                }
                                _ => {}
                            };
                            statements_checked.push(statement);
                        }
                        Err(e) => {
                            errors.extend(e);
                        }
                    }
                }

                if !found_return {
                    errors.push(ErrorInner {
                        pos: Some(pos),
                        message: format!("Expected a return statement",),
                    });
                }

                signature = Some(s);
            }
            Err(e) => {
                errors.extend(e);
            }
        };

        if errors.len() > 0 {
            return Err(errors);
        }

        self.exit_scope();

        Ok(TypedFunction {
            arguments: arguments_checked,
            statements: statements_checked,
            signature: signature.unwrap(),
        })
    }

    fn check_parameter(
        &self,
        p: ParameterNode<'ast>,
        module_id: &ModuleId,
        types: &TypeMap,
    ) -> Result<Parameter<'ast>, Vec<ErrorInner>> {
        let var = self.check_variable(p.value.id, module_id, types)?;

        Ok(Parameter {
            id: var,
            private: p.value.private,
        })
    }

    fn check_signature(
        &self,
        signature: UnresolvedSignature,
        module_id: &ModuleId,
        types: &TypeMap,
    ) -> Result<Signature, Vec<ErrorInner>> {
        let mut errors = vec![];
        let mut inputs = vec![];
        let mut outputs = vec![];

        for t in signature.inputs {
            match self.check_type(t, module_id, types) {
                Ok(t) => {
                    inputs.push(t);
                }
                Err(e) => {
                    errors.push(e);
                }
            }
        }

        for t in signature.outputs {
            match self.check_type(t, module_id, types) {
                Ok(t) => {
                    outputs.push(t);
                }
                Err(e) => {
                    errors.push(e);
                }
            }
        }

        if errors.len() > 0 {
            return Err(errors);
        }

        Ok(Signature { inputs, outputs })
    }

    fn check_type(
        &self,
        ty: UnresolvedTypeNode,
        module_id: &ModuleId,
        types: &TypeMap,
    ) -> Result<Type, ErrorInner> {
        let pos = ty.pos();
        let ty = ty.value;

        match ty {
            UnresolvedType::FieldElement => Ok(Type::FieldElement),
            UnresolvedType::Boolean => Ok(Type::Boolean),
            UnresolvedType::Uint(bitwidth) => Ok(Type::uint(bitwidth)),
            UnresolvedType::Array(t, size) => Ok(Type::Array(ArrayType::new(
                self.check_type(*t, module_id, types)?,
                size,
            ))),
            UnresolvedType::User(id) => {
                types
                    .get(module_id)
                    .unwrap()
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| ErrorInner {
                        pos: Some(pos),
                        message: format!("Undefined type {}", id),
                    })
            }
        }
    }

    fn check_variable(
        &self,
        v: crate::absy::VariableNode<'ast>,
        module_id: &ModuleId,
        types: &TypeMap,
    ) -> Result<Variable<'ast>, Vec<ErrorInner>> {
        Ok(Variable::with_id_and_type(
            v.value.id,
            self.check_type(v.value._type, module_id, types)
                .map_err(|e| vec![e])?,
        ))
    }

    fn check_statement<T: Field>(
        &mut self,
        stat: StatementNode<'ast>,
        module_id: &ModuleId,
        types: &TypeMap,
    ) -> Result<TypedStatement<'ast, T>, Vec<ErrorInner>> {
        let pos = stat.pos();

        match stat.value {
            Statement::Return(list) => {
                let mut expression_list_checked = vec![];

                for e in list.value.expressions {
                    let e_checked = self
                        .check_expression(e, module_id, &types)
                        .map_err(|e| vec![e])?;
                    expression_list_checked.push(e_checked);
                }

                Ok(TypedStatement::Return(expression_list_checked))
            }
            Statement::Declaration(var) => {
                let var = self.check_variable(var, module_id, types)?;
                match self.insert_into_scope(var.clone()) {
                    true => Ok(TypedStatement::Declaration(var)),
                    false => Err(ErrorInner {
                        pos: Some(pos),
                        message: format!("Duplicate declaration for variable named {}", var.id),
                    }),
                }
                .map_err(|e| vec![e])
            }
            Statement::Definition(assignee, expr) => {
                // we create multidef when rhs is a function call to benefit from inference
                // check rhs is not a function call here
                match expr.value {
					Expression::FunctionCall(..) => panic!("Parser should not generate Definition where the right hand side is a FunctionCall"),
					_ => {}
				}

                // check the expression to be assigned
                let checked_expr = self
                    .check_expression(expr, module_id, &types)
                    .map_err(|e| vec![e])?;
                let expression_type = checked_expr.get_type();

                // check that the assignee is declared and is well formed
                let var = self
                    .check_assignee(assignee, module_id, &types)
                    .map_err(|e| vec![e])?;

                let var_type = var.get_type();

                // make sure the assignee has the same type as the rhs
                match var_type == expression_type {
                    true => Ok(TypedStatement::Definition(var, checked_expr)),
                    false => Err(ErrorInner {
                        pos: Some(pos),
                        message: format!(
                            "Expression {} of type {} cannot be assigned to {} of type {}",
                            checked_expr, expression_type, var, var_type
                        ),
                    }),
                }
                .map_err(|e| vec![e])
            }
            Statement::Assertion(e) => {
                let e = self
                    .check_expression(e, module_id, &types)
                    .map_err(|e| vec![e])?;

                match e {
                    TypedExpression::Boolean(e) => Ok(TypedStatement::Assertion(e)),
                    e => Err(ErrorInner {
                        pos: Some(pos),
                        message: format!(
                            "Expected {} to be of type bool, found {}",
                            e,
                            e.get_type(),
                        ),
                    }),
                }
                .map_err(|e| vec![e])
            }
            Statement::For(var, from, to, statements) => {
                self.enter_scope();

                self.check_for_var(&var).map_err(|e| vec![e])?;

                let var = self.check_variable(var, module_id, types).unwrap();

                let from = self
                    .check_expression(from, module_id, &types)
                    .map_err(|e| vec![e])?;
                let to = self
                    .check_expression(to, module_id, &types)
                    .map_err(|e| vec![e])?;

                let from = match from {
                    TypedExpression::FieldElement(e) => Ok(e),
                    e => Err(ErrorInner {
                        pos: Some(pos),
                        message: format!(
                            "Expected lower loop bound to be of type field, found {}",
                            e.get_type()
                        ),
                    }),
                }
                .map_err(|e| vec![e])?;

                let to = match to {
                    TypedExpression::FieldElement(e) => Ok(e),
                    e => Err(ErrorInner {
                        pos: Some(pos),
                        message: format!(
                            "Expected higher loop bound to be of type field, found {}",
                            e.get_type()
                        ),
                    }),
                }
                .map_err(|e| vec![e])?;

                self.insert_into_scope(var.clone());

                let mut checked_statements = vec![];

                for stat in statements {
                    let checked_stat = self.check_statement(stat, module_id, types)?;
                    checked_statements.push(checked_stat);
                }

                self.exit_scope();
                Ok(TypedStatement::For(var, from, to, checked_statements))
            }
            Statement::MultipleDefinition(assignees, rhs) => {
                match rhs.value {
                    // Right side has to be a function call
                    Expression::FunctionCall(fun_id, arguments) => {

                        // check lhs assignees are defined
                        let (assignees, errors): (Vec<_>, Vec<_>) = assignees.into_iter().map(|a| self.check_assignee::<T>(a, module_id, types)).partition(|r| r.is_ok());

                        if errors.len() > 0 {
                            return Err(errors.into_iter().map(|e| e.unwrap_err()).collect());
                        }

                        let assignees: Vec<_> = assignees.into_iter().map(|a| a.unwrap()).collect();

                        let assignee_types = assignees.iter().map(|a| Some(a.get_type().clone())).collect();

                        // find argument types
                        let mut arguments_checked = vec![];
                        for arg in arguments {
                            let arg_checked = self.check_expression(arg, module_id, &types).map_err(|e| vec![e])?;
                            arguments_checked.push(arg_checked);
                        }

                        let arguments_types =
                            arguments_checked.iter().map(|a| a.get_type()).collect();

                        let query = FunctionQuery::new(&fun_id, &arguments_types, &assignee_types);

                        let f = self.find_function(&query);

                        match f {
                    		// the function has to be defined
                    		Some(f) => {

                                let call = TypedExpressionList::FunctionCall(f.clone(), arguments_checked, f.signature.outputs.clone());

                                Ok(TypedStatement::MultipleDefinition(assignees, call))
                    		},
                    		None => Err(ErrorInner {                         pos: Some(pos),
 message: format!("Function definition for function {} with signature {} not found.", fun_id, query) }),
                    	}
                    }
                    _ => Err(ErrorInner {
                        pos: Some(pos),
                        message: format!("{} should be a function call", rhs),
                    }),
                }.map_err(|e| vec![e])
            }
        }
    }

    fn check_assignee<T: Field>(
        &mut self,
        assignee: AssigneeNode<'ast>,
        module_id: &ModuleId,
        types: &TypeMap,
    ) -> Result<TypedAssignee<'ast, T>, ErrorInner> {
        let pos = assignee.pos();
        // check that the assignee is declared
        match assignee.value {
            Assignee::Identifier(variable_name) => match self.get_scope(&variable_name) {
                Some(var) => Ok(TypedAssignee::Identifier(Variable::with_id_and_type(
                    variable_name,
                    var.id._type.clone(),
                ))),
                None => Err(ErrorInner {
                    pos: Some(assignee.pos()),
                    message: format!("Variable `{}` is undeclared", variable_name),
                }),
            },
            Assignee::Select(box assignee, box index) => {
                let checked_assignee = self.check_assignee(assignee, module_id, &types)?;

                let ty = checked_assignee.get_type();
                match ty {
                    Type::Array(..) => {
                        let checked_index = match index {
                            RangeOrExpression::Expression(e) => {
                                self.check_expression(e, module_id, &types)?
                            }
                            r => unimplemented!(
                                "Using slices in assignments is not supported yet, found {}",
                                r
                            ),
                        };

                        let checked_typed_index = match checked_index {
                            TypedExpression::FieldElement(e) => Ok(e),
                            e => Err(ErrorInner {
                                pos: Some(pos),

                                message: format!(
                                    "Expected array {} index to have type field, found {}",
                                    checked_assignee,
                                    e.get_type()
                                ),
                            }),
                        }?;

                        Ok(TypedAssignee::Select(
                            box checked_assignee,
                            box checked_typed_index,
                        ))
                    }
                    ty => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!(
                            "Cannot access element at index {} on {} of type {}",
                            index, checked_assignee, ty,
                        ),
                    }),
                }
            }
            Assignee::Member(box assignee, box member) => {
                let checked_assignee = self.check_assignee(assignee, module_id, &types)?;

                let ty = checked_assignee.get_type();
                match &ty {
                    Type::Struct(members) => match members.iter().find(|m| m.id == member) {
                        Some(_) => Ok(TypedAssignee::Member(box checked_assignee, member.into())),
                        None => Err(ErrorInner {
                            pos: Some(pos),
                            message: format!("{} doesn't have member {}", ty, member),
                        }),
                    },
                    ty => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!(
                            "Cannot access field {} on {} as of type {}",
                            member, checked_assignee, ty,
                        ),
                    }),
                }
            }
        }
    }

    fn check_spread_or_expression<T: Field>(
        &mut self,
        spread_or_expression: SpreadOrExpression<'ast>,
        module_id: &ModuleId,
        types: &TypeMap,
    ) -> Result<Vec<TypedExpression<'ast, T>>, ErrorInner> {
        match spread_or_expression {
            SpreadOrExpression::Spread(s) => {
                let pos = s.pos();

                let checked_expression =
                    self.check_expression(s.value.expression, module_id, &types)?;

                let res = match checked_expression {
                    TypedExpression::Array(e) => {
                        let ty = e.inner_type().clone();

                        let size = e.size();
                        match e.into_inner() {
                            // if we're doing a spread over an inline array, we return the inside of the array: ...[x, y, z] == x, y, z
                            // this is not strictly needed, but it makes spreads memory linear rather than quadratic
                            ArrayExpressionInner::Value(v) => Ok(v),
                            // otherwise we return a[0], ..., a[a.size() -1 ]
                            e => Ok((0..size)
                                .map(|i| match &ty {
                                    Type::FieldElement => FieldElementExpression::select(
                                        e.clone().annotate(Type::FieldElement, size),
                                        FieldElementExpression::Number(T::from(i)),
                                    )
                                    .into(),
                                    Type::Uint(bitwidth) => UExpression::select(
                                        e.clone().annotate(Type::Uint(*bitwidth), size),
                                        FieldElementExpression::Number(T::from(i)),
                                    )
                                    .into(),
                                    Type::Boolean => BooleanExpression::select(
                                        e.clone().annotate(Type::Boolean, size),
                                        FieldElementExpression::Number(T::from(i)),
                                    )
                                    .into(),
                                    Type::Array(array_type) => ArrayExpressionInner::Select(
                                        box e
                                            .clone()
                                            .annotate(Type::Array(array_type.clone()), size),
                                        box FieldElementExpression::Number(T::from(i)),
                                    )
                                    .annotate(*array_type.ty.clone(), array_type.size)
                                    .into(),
                                    Type::Struct(members) => StructExpressionInner::Select(
                                        box e.clone().annotate(Type::Struct(members.clone()), size),
                                        box FieldElementExpression::Number(T::from(i)),
                                    )
                                    .annotate(members.clone())
                                    .into(),
                                })
                                .collect()),
                        }
                    }
                    e => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!(
                            "Expected spread operator to apply on array, found {}",
                            e.get_type()
                        ),
                    }),
                };

                let res = res.unwrap();

                Ok(res)
            }
            SpreadOrExpression::Expression(e) => {
                self.check_expression(e, module_id, &types).map(|r| vec![r])
            }
        }
    }

    fn check_expression<T: Field>(
        &mut self,
        expr: ExpressionNode<'ast>,
        module_id: &ModuleId,
        types: &TypeMap,
    ) -> Result<TypedExpression<'ast, T>, ErrorInner> {
        let pos = expr.pos();

        match expr.value {
            Expression::BooleanConstant(b) => Ok(BooleanExpression::Value(b).into()),
            Expression::Identifier(name) => {
                // check that `id` is defined in the scope
                match self.get_scope(&name) {
                    Some(v) => match v.id.get_type() {
                        Type::Boolean => Ok(BooleanExpression::Identifier(name.into()).into()),
                        Type::Uint(bitwidth) => Ok(UExpressionInner::Identifier(name.into())
                            .annotate(bitwidth)
                            .into()),
                        Type::FieldElement => {
                            Ok(FieldElementExpression::Identifier(name.into()).into())
                        }
                        Type::Array(array_type) => {
                            Ok(ArrayExpressionInner::Identifier(name.into())
                                .annotate(*array_type.ty, array_type.size)
                                .into())
                        }
                        Type::Struct(members) => Ok(StructExpressionInner::Identifier(name.into())
                            .annotate(members)
                            .into()),
                    },
                    None => Err(ErrorInner {
                        pos: Some(pos),
                        message: format!("Identifier \"{}\" is undefined", name),
                    }),
                }
            }
            Expression::Add(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;

                match (e1_checked, e2_checked) {
                    (TypedExpression::FieldElement(e1), TypedExpression::FieldElement(e2)) => {
                        Ok(FieldElementExpression::Add(box e1, box e2).into())
                    }
                    (TypedExpression::Uint(e1), TypedExpression::Uint(e2)) => {
                        if e1.get_type() == e2.get_type() {
                            Ok(UExpression::add(e1, e2).into())
                        } else {
                            Err(ErrorInner {
                                pos: Some(pos),

                                message: format!(
                                    "Cannot apply `+` to {}, {}",
                                    e1.get_type(),
                                    e2.get_type()
                                ),
                            })
                        }
                    }
                    (t1, t2) => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!(
                            "Cannot apply `+` to {}, {}",
                            t1.get_type(),
                            t2.get_type()
                        ),
                    }),
                }
            }
            Expression::Sub(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;

                match (e1_checked, e2_checked) {
                    (TypedExpression::FieldElement(e1), TypedExpression::FieldElement(e2)) => {
                        Ok(FieldElementExpression::Sub(box e1, box e2).into())
                    }
                    (TypedExpression::Uint(e1), TypedExpression::Uint(e2)) => {
                        if e1.get_type() == e2.get_type() {
                            Ok(UExpression::sub(e1, e2).into())
                        } else {
                            Err(ErrorInner {
                                pos: Some(pos),

                                message: format!(
                                    "Cannot apply `+` to {}, {}",
                                    e1.get_type(),
                                    e2.get_type()
                                ),
                            })
                        }
                    }
                    (t1, t2) => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!(
                            "Expected only field elements, found {}, {}",
                            t1.get_type(),
                            t2.get_type()
                        ),
                    }),
                }
            }
            Expression::Mult(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;

                match (e1_checked, e2_checked) {
                    (TypedExpression::FieldElement(e1), TypedExpression::FieldElement(e2)) => {
                        Ok(FieldElementExpression::Mult(box e1, box e2).into())
                    }
                    (TypedExpression::Uint(e1), TypedExpression::Uint(e2)) => {
                        if e1.get_type() == e2.get_type() {
                            Ok(UExpression::mult(e1, e2).into())
                        } else {
                            Err(ErrorInner {
                                pos: Some(pos),

                                message: format!(
                                    "Cannot apply `*` to {}, {}",
                                    e1.get_type(),
                                    e2.get_type()
                                ),
                            })
                        }
                    }
                    (t1, t2) => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!(
                            "Cannot apply `*` to {}, {}",
                            t1.get_type(),
                            t2.get_type()
                        ),
                    }),
                }
            }
            Expression::Div(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;

                match (e1_checked, e2_checked) {
                    (TypedExpression::FieldElement(e1), TypedExpression::FieldElement(e2)) => {
                        Ok(FieldElementExpression::Div(box e1, box e2).into())
                    }
                    (TypedExpression::Uint(e1), TypedExpression::Uint(e2)) => {
                        if e1.get_type() == e2.get_type() {
                            Ok(UExpression::div(e1, e2).into())
                        } else {
                            Err(ErrorInner {
                                pos: Some(pos),

                                message: format!(
                                    "Cannot apply `/` to {}, {}",
                                    e1.get_type(),
                                    e2.get_type()
                                ),
                            })
                        }
                    }
                    (t1, t2) => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!(
                            "Cannot apply `/` to {}, {}",
                            t1.get_type(),
                            t2.get_type()
                        ),
                    }),
                }
            }
            Expression::Rem(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;

                match (e1_checked, e2_checked) {
                    (TypedExpression::Uint(e1), TypedExpression::Uint(e2)) => {
                        if e1.get_type() == e2.get_type() {
                            Ok(UExpression::rem(e1, e2).into())
                        } else {
                            Err(ErrorInner {
                                pos: Some(pos),

                                message: format!(
                                    "Cannot apply `%` to {}, {}",
                                    e1.get_type(),
                                    e2.get_type()
                                ),
                            })
                        }
                    }
                    (t1, t2) => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!(
                            "Cannot apply `%` to {}, {}",
                            t1.get_type(),
                            t2.get_type()
                        ),
                    }),
                }
            }
            Expression::Pow(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;

                match (e1_checked, e2_checked) {
                    (TypedExpression::FieldElement(e1), TypedExpression::FieldElement(e2)) => Ok(
                        TypedExpression::FieldElement(FieldElementExpression::Pow(box e1, box e2)),
                    ),
                    (t1, t2) => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!(
                            "Expected only field elements, found {}, {}",
                            t1.get_type(),
                            t2.get_type()
                        ),
                    }),
                }
            }
            Expression::IfElse(box condition, box consequence, box alternative) => {
                let condition_checked = self.check_expression(condition, module_id, &types)?;
                let consequence_checked = self.check_expression(consequence, module_id, &types)?;
                let alternative_checked = self.check_expression(alternative, module_id, &types)?;

                match condition_checked {
                    TypedExpression::Boolean(condition) => {
                        let consequence_type = consequence_checked.get_type();
                        let alternative_type = alternative_checked.get_type();
                        match consequence_type == alternative_type {
                            true => match (consequence_checked, alternative_checked) {
                                (TypedExpression::FieldElement(consequence), TypedExpression::FieldElement(alternative)) => {
                                    Ok(FieldElementExpression::IfElse(box condition, box consequence, box alternative).into())
                                },
                                (TypedExpression::Boolean(consequence), TypedExpression::Boolean(alternative)) => {
                                    Ok(BooleanExpression::IfElse(box condition, box consequence, box alternative).into())
                                },
                                (TypedExpression::Array(consequence), TypedExpression::Array(alternative)) => {
                                    let inner_type = consequence.inner_type().clone();
                                    let size = consequence.size();
                                    Ok(ArrayExpressionInner::IfElse(box condition, box consequence, box alternative).annotate(inner_type, size).into())
                                },
                                (TypedExpression::Struct(consequence), TypedExpression::Struct(alternative)) => {
                                    let ty = consequence.ty().clone();
                                    Ok(StructExpressionInner::IfElse(box condition, box consequence, box alternative).annotate(ty).into())
                                },
                                (TypedExpression::Uint(consequence), TypedExpression::Uint(alternative)) => {
                                    let bitwidth = consequence.bitwidth();
                                    Ok(UExpressionInner::IfElse(box condition, box consequence, box alternative).annotate(bitwidth).into())
                                },
                                _ => unreachable!("types should match here as we checked them explicitly")
                            }
                            false => Err(ErrorInner {
                                pos: Some(pos),
                                message: format!("{{consequence}} and {{alternative}} in `if/else` expression should have the same type, found {}, {}", consequence_type, alternative_type)
                            })
                        }
                    }
                    c => Err(ErrorInner {
                        pos: Some(pos),
                        message: format!(
                            "{{condition}} after `if` should be a boolean, found {}",
                            c.get_type()
                        ),
                    }),
                }
            }
            Expression::FieldConstant(n) => Ok(FieldElementExpression::Number(
                T::try_from(n).map_err(|_| ErrorInner {
                    pos: Some(pos),
                    message: format!(
                        "Field constant not in the representable range [{}, {}]",
                        T::min_value(),
                        T::max_value()
                    ),
                })?,
            )
            .into()),
            Expression::U8Constant(n) => Ok(UExpressionInner::Value(n.into()).annotate(8).into()),
            Expression::U16Constant(n) => Ok(UExpressionInner::Value(n.into()).annotate(16).into()),
            Expression::U32Constant(n) => Ok(UExpressionInner::Value(n.into()).annotate(32).into()),
            Expression::FunctionCall(fun_id, arguments) => {
                // check the arguments
                let mut arguments_checked = vec![];
                for arg in arguments {
                    let arg_checked = self.check_expression(arg, module_id, &types)?;
                    arguments_checked.push(arg_checked);
                }

                let mut arguments_types = vec![];
                for arg in arguments_checked.iter() {
                    arguments_types.push(arg.get_type());
                }

                // outside of multidef, function calls must have a single return value
                // we use type inference to determine the type of the return, so we don't specify it
                let query = FunctionQuery::new(&fun_id, &arguments_types, &vec![None]);

                let f = self.find_function(&query);

                match f {
                    // the function has to be defined
                    Some(f) => {
                        // the return count has to be 1
                        match f.signature.outputs.len() {
                            1 => match &f.signature.outputs[0] {
                                Type::FieldElement => Ok(FieldElementExpression::FunctionCall(
                                    FunctionKey {
                                        id: f.id.clone(),
                                        signature: f.signature.clone(),
                                    },
                                    arguments_checked,
                                )
                                .into()),
                                Type::Boolean => Ok(BooleanExpression::FunctionCall(
                                    FunctionKey {
                                        id: f.id.clone(),
                                        signature: f.signature.clone(),
                                    },
                                    arguments_checked,
                                )
                                .into()),
                                Type::Uint(bitwidth) => Ok(UExpressionInner::FunctionCall(
                                    FunctionKey {
                                        id: f.id.clone(),
                                        signature: f.signature.clone(),
                                    },
                                    arguments_checked,
                                )
                                .annotate(*bitwidth)
                                .into()),
                                Type::Struct(members) => Ok(StructExpressionInner::FunctionCall(
                                    FunctionKey {
                                        id: f.id.clone(),
                                        signature: f.signature.clone(),
                                    },
                                    arguments_checked,
                                )
                                .annotate(members.clone())
                                .into()),
                                Type::Array(array_type) => Ok(ArrayExpressionInner::FunctionCall(
                                    FunctionKey {
                                        id: f.id.clone(),
                                        signature: f.signature.clone(),
                                    },
                                    arguments_checked,
                                )
                                .annotate(*array_type.ty.clone(), array_type.size.clone())
                                .into()),
                            },
                            n => Err(ErrorInner {
                                pos: Some(pos),

                                message: format!(
                                    "{} returns {} values but is called outside of a definition",
                                    f.id, n
                                ),
                            }),
                        }
                    }
                    None => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!(
                            "Function definition for function {} with signature {} not found.",
                            fun_id, query
                        ),
                    }),
                }
            }
            Expression::Lt(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;
                match (e1_checked, e2_checked) {
                    (TypedExpression::FieldElement(e1), TypedExpression::FieldElement(e2)) => {
                        Ok(BooleanExpression::Lt(box e1, box e2).into())
                    }
                    (e1, e2) => Err(ErrorInner {
                        pos: Some(pos),
                        message: format!(
                            "Cannot compare {} of type {} to {} of type {}",
                            e1,
                            e1.get_type(),
                            e2,
                            e2.get_type()
                        ),
                    }),
                }
            }
            Expression::Le(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;
                match (e1_checked, e2_checked) {
                    (TypedExpression::FieldElement(e1), TypedExpression::FieldElement(e2)) => {
                        Ok(BooleanExpression::Le(box e1, box e2).into())
                    }
                    (e1, e2) => Err(ErrorInner {
                        pos: Some(pos),
                        message: format!(
                            "Cannot compare {} of type {} to {} of type {}",
                            e1,
                            e1.get_type(),
                            e2,
                            e2.get_type()
                        ),
                    }),
                }
            }
            Expression::Eq(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;
                match (e1_checked, e2_checked) {
                    (TypedExpression::FieldElement(e1), TypedExpression::FieldElement(e2)) => {
                        Ok(BooleanExpression::FieldEq(box e1, box e2).into())
                    }
                    (TypedExpression::Boolean(e1), TypedExpression::Boolean(e2)) => {
                        Ok(BooleanExpression::BoolEq(box e1, box e2).into())
                    }
                    (TypedExpression::Array(e1), TypedExpression::Array(e2)) => {
                        if e1.get_type() == e2.get_type() {
                            Ok(BooleanExpression::ArrayEq(box e1, box e2).into())
                        } else {
                            Err(ErrorInner {
                                pos: Some(pos),
                                message: format!(
                                    "Cannot compare {} of type {} to {} of type {}",
                                    e1,
                                    e1.get_type(),
                                    e2,
                                    e2.get_type()
                                ),
                            })
                        }
                    }
                    (TypedExpression::Struct(e1), TypedExpression::Struct(e2)) => {
                        if e1.get_type() == e2.get_type() {
                            Ok(BooleanExpression::StructEq(box e1, box e2).into())
                        } else {
                            Err(ErrorInner {
                                pos: Some(pos),
                                message: format!(
                                    "Cannot compare {} of type {} to {} of type {}",
                                    e1,
                                    e1.get_type(),
                                    e2,
                                    e2.get_type()
                                ),
                            })
                        }
                    }
                    (TypedExpression::Uint(e1), TypedExpression::Uint(e2)) => {
                        if e1.get_type() == e2.get_type() {
                            Ok(BooleanExpression::UintEq(box e1, box e2).into())
                        } else {
                            Err(ErrorInner {
                                pos: Some(pos),
                                message: format!(
                                    "Cannot compare {} of type {} to {} of type {}",
                                    e1,
                                    e1.get_type(),
                                    e2,
                                    e2.get_type()
                                ),
                            })
                        }
                    }
                    (e1, e2) => Err(ErrorInner {
                        pos: Some(pos),
                        message: format!(
                            "Cannot compare {} of type {} to {} of type {}",
                            e1,
                            e1.get_type(),
                            e2,
                            e2.get_type()
                        ),
                    }),
                }
            }
            Expression::Ge(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;
                match (e1_checked, e2_checked) {
                    (TypedExpression::FieldElement(e1), TypedExpression::FieldElement(e2)) => {
                        Ok(BooleanExpression::Ge(box e1, box e2).into())
                    }
                    (e1, e2) => Err(ErrorInner {
                        pos: Some(pos),
                        message: format!(
                            "Cannot compare {} of type {} to {} of type {}",
                            e1,
                            e1.get_type(),
                            e2,
                            e2.get_type()
                        ),
                    }),
                }
            }
            Expression::Gt(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;
                match (e1_checked, e2_checked) {
                    (TypedExpression::FieldElement(e1), TypedExpression::FieldElement(e2)) => {
                        Ok(BooleanExpression::Gt(box e1, box e2).into())
                    }
                    (e1, e2) => Err(ErrorInner {
                        pos: Some(pos),
                        message: format!(
                            "Cannot compare {} of type {} to {} of type {}",
                            e1,
                            e1.get_type(),
                            e2,
                            e2.get_type()
                        ),
                    }),
                }
            }
            Expression::Select(box array, box index) => {
                let array = self.check_expression(array, module_id, &types)?;

                match index {
                    RangeOrExpression::Range(r) => match array {
                        TypedExpression::Array(array) => {
                            let array_size = array.size();
                            let inner_type = array.inner_type().clone();

                            // check that the bounds are valid expressions
                            let from = r
                                .value
                                .from
                                .map(|e| self.check_expression(e, module_id, &types))
                                .unwrap_or(Ok(FieldElementExpression::Number(T::from(0)).into()))?;

                            let to = r
                                .value
                                .to
                                .map(|e| self.check_expression(e, module_id, &types))
                                .unwrap_or(Ok(FieldElementExpression::Number(T::from(
                                    array_size,
                                ))
                                .into()))?;

                            // check the bounds are field constants
                            // Note: it would be nice to allow any field expression, and check it's a constant after constant propagation,
                            // but it's tricky from a type perspective: the size of the slice changes the type of the resulting array,
                            // which doesn't work well with our static array approach. Enabling arrays to have unknown size introduces a lot
                            // of complexity in the compiler, as function selection in inlining requires knowledge of the array size, but
                            // determining array size potentially requires inlining and propagating. This suggests we would need semantic checking
                            // to happen iteratively with inlining and propagation, which we can't do now as we go from absy to typed_absy
                            let from = match from {
                                TypedExpression::FieldElement(FieldElementExpression::Number(n)) => Ok(n.to_dec_string().parse::<usize>().unwrap()),
                                e => Err(ErrorInner {
                                    pos: Some(pos),
                                    message: format!(
                                        "Expected the lower bound of the range to be a constant field, found {}",
                                        e
                                    ),
                                })
                            }?;

                            let to = match to {
                                TypedExpression::FieldElement(FieldElementExpression::Number(n)) => Ok(n.to_dec_string().parse::<usize>().unwrap()),
                                e => Err(ErrorInner {
                                    pos: Some(pos),
                                    message: format!(
                                        "Expected the higher bound of the range to be a constant field, found {}",
                                        e
                                    ),
                                })
                            }?;

                            match (from, to, array_size) {
                                (f, _, s) if f > s => Err(ErrorInner {
                                    pos: Some(pos),
                                    message: format!(
                                        "Lower range bound {} is out of array bounds [0, {}]",
                                        f, s,
                                    ),
                                }),
                                (_, t, s) if t > s => Err(ErrorInner {
                                    pos: Some(pos),
                                    message: format!(
                                        "Higher range bound {} is out of array bounds [0, {}]",
                                        t, s,
                                    ),
                                }),
                                (f, t, _) if f > t => Err(ErrorInner {
                                    pos: Some(pos),
                                    message: format!(
                                        "Lower range bound {} is larger than higher range bound {}",
                                        f, t,
                                    ),
                                }),
                                (f, t, _) => Ok(ArrayExpressionInner::Value(
                                    (f..t)
                                        .map(|i| match inner_type.clone() {
                                            Type::FieldElement => FieldElementExpression::Select(
                                                box array.clone(),
                                                box FieldElementExpression::Number(T::from(i)),
                                            )
                                            .into(),
                                            Type::Boolean => BooleanExpression::Select(
                                                box array.clone(),
                                                box FieldElementExpression::Number(T::from(i)),
                                            )
                                            .into(),
                                            Type::Uint(bitwidth) => UExpressionInner::Select(
                                                box array.clone(),
                                                box FieldElementExpression::Number(T::from(i)),
                                            )
                                            .annotate(bitwidth)
                                            .into(),
                                            Type::Struct(struct_ty) => {
                                                StructExpressionInner::Select(
                                                    box array.clone(),
                                                    box FieldElementExpression::Number(T::from(i)),
                                                )
                                                .annotate(struct_ty)
                                                .into()
                                            }
                                            Type::Array(array_ty) => ArrayExpressionInner::Select(
                                                box array.clone(),
                                                box FieldElementExpression::Number(T::from(i)),
                                            )
                                            .annotate(*array_ty.ty, array_ty.size)
                                            .into(),
                                        })
                                        .collect(),
                                )
                                .annotate(inner_type, t - f)
                                .into()),
                            }
                        }
                        e => Err(ErrorInner {
                            pos: Some(pos),
                            message: format!(
                                "Cannot access slice of expression {} of type {}",
                                e,
                                e.get_type(),
                            ),
                        }),
                    },
                    RangeOrExpression::Expression(e) => {
                        match (array, self.check_expression(e, module_id, &types)?) {
                            (TypedExpression::Array(a), TypedExpression::FieldElement(i)) => {
                                match a.inner_type().clone() {
                                    Type::FieldElement => {
                                        Ok(FieldElementExpression::select(a, i).into())
                                    }
                                    Type::Uint(..) => Ok(UExpression::select(a, i).into()),
                                    Type::Boolean => Ok(BooleanExpression::select(a, i).into()),
                                    Type::Array(..) => Ok(ArrayExpression::select(a, i).into()),
                                    Type::Struct(..) => Ok(StructExpression::select(a, i).into()),
                                }
                            }
                            (a, e) => Err(ErrorInner {
                                pos: Some(pos),
                                message: format!(
                                    "Cannot access element {} on expression of type {}",
                                    e,
                                    a.get_type()
                                ),
                            }),
                        }
                    }
                }
            }
            Expression::Member(box e, box id) => {
                let e = self.check_expression(e, module_id, &types)?;

                match e {
                    TypedExpression::Struct(s) => {
                        // check that the struct has that field and return the type if it does
                        let ty = s.ty().iter().find(|m| m.id == id).map(|m| *m.ty.clone());

                        match ty {
                            Some(ty) => match ty {
                                Type::FieldElement => {
                                    Ok(FieldElementExpression::member(s, id.to_string()).into())
                                }
                                Type::Boolean => {
                                    Ok(BooleanExpression::member(s, id.to_string()).into())
                                }
                                Type::Uint(..) => Ok(UExpression::member(s, id.to_string()).into()),
                                Type::Array(array_type) => {
                                    Ok(ArrayExpressionInner::Member(box s.clone(), id.to_string())
                                        .annotate(*array_type.ty.clone(), array_type.size)
                                        .into())
                                }
                                Type::Struct(..) => {
                                    Ok(StructExpression::member(s.clone(), id.to_string()).into())
                                }
                            },
                            None => Err(ErrorInner {
                                pos: Some(pos),
                                message: format!("{} doesn't have member {}", s.get_type(), id,),
                            }),
                        }
                    }
                    e => Err(ErrorInner {
                        pos: Some(pos),
                        message: format!(
                            "Cannot access member {} on expression of type {}",
                            id,
                            e.get_type()
                        ),
                    }),
                }
            }
            Expression::InlineArray(expressions) => {
                // check each expression, getting its type
                let mut expressions_checked = vec![];
                for e in expressions {
                    let e_checked = self.check_spread_or_expression(e, module_id, &types)?;
                    expressions_checked.extend(e_checked);
                }

                // we infer the type to be the type of the first element
                let inferred_type = expressions_checked.get(0).unwrap().get_type().clone();

                match inferred_type {
                    Type::FieldElement => {
                        // we check all expressions have that same type
                        let mut unwrapped_expressions = vec![];

                        for e in expressions_checked {
                            let unwrapped_e = match e {
                                TypedExpression::FieldElement(e) => Ok(e),
                                e => Err(ErrorInner {
                                    pos: Some(pos),

                                    message: format!(
                                        "Expected {} to have type {}, but type is {}",
                                        e,
                                        inferred_type,
                                        e.get_type()
                                    ),
                                }),
                            }?;
                            unwrapped_expressions.push(unwrapped_e.into());
                        }

                        let size = unwrapped_expressions.len();

                        Ok(ArrayExpressionInner::Value(unwrapped_expressions)
                            .annotate(Type::FieldElement, size)
                            .into())
                    }
                    Type::Boolean => {
                        // we check all expressions have that same type
                        let mut unwrapped_expressions = vec![];

                        for e in expressions_checked {
                            let unwrapped_e = match e {
                                TypedExpression::Boolean(e) => Ok(e),
                                e => Err(ErrorInner {
                                    pos: Some(pos),

                                    message: format!(
                                        "Expected {} to have type {}, but type is {}",
                                        e,
                                        inferred_type,
                                        e.get_type()
                                    ),
                                }),
                            }?;
                            unwrapped_expressions.push(unwrapped_e.into());
                        }

                        let size = unwrapped_expressions.len();

                        Ok(ArrayExpressionInner::Value(unwrapped_expressions)
                            .annotate(Type::Boolean, size)
                            .into())
                    }
                    ty @ Type::Uint(..) => {
                        // we check all expressions have that same type
                        let mut unwrapped_expressions = vec![];

                        for e in expressions_checked {
                            let unwrapped_e = match e {
                                TypedExpression::Uint(e) => {
                                    if e.get_type() == ty {
                                        Ok(e)
                                    } else {
                                        Err(ErrorInner {
                                            pos: Some(pos),

                                            message: format!(
                                                "Expected {} to have type {}, but type is {}",
                                                e,
                                                ty,
                                                e.get_type()
                                            ),
                                        })
                                    }
                                }
                                e => Err(ErrorInner {
                                    pos: Some(pos),

                                    message: format!(
                                        "Expected {} to have type {}, but type is {}",
                                        e,
                                        ty,
                                        e.get_type()
                                    ),
                                }),
                            }?;
                            unwrapped_expressions.push(unwrapped_e.into());
                        }

                        let size = unwrapped_expressions.len();

                        Ok(ArrayExpressionInner::Value(unwrapped_expressions)
                            .annotate(ty, size)
                            .into())
                    }
                    ty @ Type::Array(..) => {
                        // we check all expressions have that same type
                        let mut unwrapped_expressions = vec![];

                        for e in expressions_checked {
                            let unwrapped_e = match e {
                                TypedExpression::Array(e) => {
                                    if e.get_type() == ty {
                                        Ok(e)
                                    } else {
                                        Err(ErrorInner {
                                            pos: Some(pos),

                                            message: format!(
                                                "Expected {} to have type {}, but type is {}",
                                                e,
                                                ty,
                                                e.get_type()
                                            ),
                                        })
                                    }
                                }
                                e => Err(ErrorInner {
                                    pos: Some(pos),

                                    message: format!(
                                        "Expected {} to have type {}, but type is {}",
                                        e,
                                        ty,
                                        e.get_type()
                                    ),
                                }),
                            }?;
                            unwrapped_expressions.push(unwrapped_e.into());
                        }

                        let size = unwrapped_expressions.len();

                        Ok(ArrayExpressionInner::Value(unwrapped_expressions)
                            .annotate(ty, size)
                            .into())
                    }
                    ty @ Type::Struct(..) => {
                        // we check all expressions have that same type
                        let mut unwrapped_expressions = vec![];

                        for e in expressions_checked {
                            let unwrapped_e = match e {
                                TypedExpression::Struct(e) => {
                                    if e.get_type() == ty {
                                        Ok(e)
                                    } else {
                                        Err(ErrorInner {
                                            pos: Some(pos),

                                            message: format!(
                                                "Expected {} to have type {}, but type is {}",
                                                e,
                                                ty,
                                                e.get_type()
                                            ),
                                        })
                                    }
                                }
                                e => Err(ErrorInner {
                                    pos: Some(pos),

                                    message: format!(
                                        "Expected {} to have type {}, but type is {}",
                                        e,
                                        ty,
                                        e.get_type()
                                    ),
                                }),
                            }?;
                            unwrapped_expressions.push(unwrapped_e.into());
                        }

                        let size = unwrapped_expressions.len();

                        Ok(ArrayExpressionInner::Value(unwrapped_expressions)
                            .annotate(ty, size)
                            .into())
                    }
                }
            }
            Expression::InlineStruct(id, inline_members) => {
                let ty = self.check_type(
                    UnresolvedType::User(id.clone()).at(42, 42, 42),
                    module_id,
                    &types,
                )?;
                let struct_type = match ty {
                    Type::Struct(struct_type) => struct_type,
                    _ => unreachable!(),
                };

                // check that we provided the required number of values

                if struct_type.len() != inline_members.len() {
                    return Err(ErrorInner {
                        pos: Some(pos),
                        message: format!(
                            "Inline struct {} does not match {}",
                            Expression::InlineStruct(id.clone(), inline_members),
                            Type::Struct(struct_type)
                        ),
                    });
                }

                // check that the mapping of values matches the expected type
                // put the value into a map, pick members from this map following declared members, and try to parse them

                let mut inline_members_map = inline_members
                    .clone()
                    .into_iter()
                    .map(|(id, v)| (id.to_string(), v))
                    .collect::<HashMap<_, _>>();
                let mut result: Vec<TypedExpression<'ast, T>> = vec![];

                for member in struct_type.iter() {
                    match inline_members_map.remove(member.id.as_str()) {
                        Some(value) => {
                            let expression_checked =
                                self.check_expression(value, module_id, &types)?;
                            let checked_type = expression_checked.get_type();
                            if checked_type != *member.ty {
                                return Err(ErrorInner {
                                    pos: Some(pos),
                                    message: format!(
                                        "Member {} of struct {} has type {}, found {} of type {}",
                                        member.id,
                                        id.clone(),
                                        member.ty,
                                        expression_checked,
                                        checked_type,
                                    ),
                                });
                            } else {
                                result.push(expression_checked.into());
                            }
                        }
                        None => {
                            return Err(ErrorInner {
                                pos: Some(pos),
                                message: format!(
                                    "Member {} of struct {} not found in value {}",
                                    member.id,
                                    Type::Struct(struct_type.clone()),
                                    Expression::InlineStruct(id.clone(), inline_members),
                                ),
                            })
                        }
                    }
                }

                Ok(StructExpressionInner::Value(result)
                    .annotate(struct_type)
                    .into())
            }
            Expression::And(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;
                match (e1_checked, e2_checked) {
                    (TypedExpression::Boolean(e1), TypedExpression::Boolean(e2)) => {
                        Ok(BooleanExpression::And(box e1, box e2).into())
                    }
                    (e1, e2) => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!(
                            "cannot apply boolean operators to {} and {}",
                            e1.get_type(),
                            e2.get_type()
                        ),
                    }),
                }
            }
            Expression::Or(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;
                match (e1_checked, e2_checked) {
                    (TypedExpression::Boolean(e1), TypedExpression::Boolean(e2)) => {
                        Ok(BooleanExpression::Or(box e1, box e2).into())
                    }
                    (e1, e2) => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!("cannot compare {} to {}", e1.get_type(), e2.get_type()),
                    }),
                }
            }
            Expression::LeftShift(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;
                match (e1_checked, e2_checked) {
                    (TypedExpression::Uint(e1), TypedExpression::FieldElement(e2)) => {
                        Ok(UExpression::left_shift(e1, e2).into())
                    }
                    (e1, e2) => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!(
                            "cannot left-shift {} by {}",
                            e1.get_type(),
                            e2.get_type()
                        ),
                    }),
                }
            }
            Expression::RightShift(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;
                match (e1_checked, e2_checked) {
                    (TypedExpression::Uint(e1), TypedExpression::FieldElement(e2)) => {
                        Ok(UExpression::right_shift(e1, e2).into())
                    }
                    (e1, e2) => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!(
                            "cannot right-shift {} by {}",
                            e1.get_type(),
                            e2.get_type()
                        ),
                    }),
                }
            }
            Expression::BitOr(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;
                match (e1_checked, e2_checked) {
                    (TypedExpression::Uint(e1), TypedExpression::Uint(e2)) => {
                        if e1.get_type() == e2.get_type() {
                            Ok(UExpression::or(e1, e2).into())
                        } else {
                            Err(ErrorInner {
                                pos: Some(pos),

                                message: format!(
                                    "Cannot apply `|` to {}, {}",
                                    e1.get_type(),
                                    e2.get_type()
                                ),
                            })
                        }
                    }
                    (e1, e2) => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!(
                            "Cannot apply `|` to {}, {}",
                            e1.get_type(),
                            e2.get_type()
                        ),
                    }),
                }
            }
            Expression::BitAnd(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;
                match (e1_checked, e2_checked) {
                    (TypedExpression::Uint(e1), TypedExpression::Uint(e2)) => {
                        if e1.get_type() == e2.get_type() {
                            Ok(UExpression::and(e1, e2).into())
                        } else {
                            Err(ErrorInner {
                                pos: Some(pos),

                                message: format!(
                                    "Cannot apply `&` to {}, {}",
                                    e1.get_type(),
                                    e2.get_type()
                                ),
                            })
                        }
                    }
                    (e1, e2) => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!(
                            "Cannot apply `&` to {}, {}",
                            e1.get_type(),
                            e2.get_type()
                        ),
                    }),
                }
            }
            Expression::BitXor(box e1, box e2) => {
                let e1_checked = self.check_expression(e1, module_id, &types)?;
                let e2_checked = self.check_expression(e2, module_id, &types)?;
                match (e1_checked, e2_checked) {
                    (TypedExpression::Uint(e1), TypedExpression::Uint(e2)) => {
                        if e1.get_type() == e2.get_type() {
                            Ok(UExpression::xor(e1, e2).into())
                        } else {
                            Err(ErrorInner {
                                pos: Some(pos),

                                message: format!(
                                    "Cannot apply `^` to {}, {}",
                                    e1.get_type(),
                                    e2.get_type()
                                ),
                            })
                        }
                    }
                    (e1, e2) => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!(
                            "Cannot apply `^` to {}, {}",
                            e1.get_type(),
                            e2.get_type()
                        ),
                    }),
                }
            }
            Expression::Not(box e) => {
                let e_checked = self.check_expression(e, module_id, &types)?;
                match e_checked {
                    TypedExpression::Boolean(e) => Ok(BooleanExpression::Not(box e).into()),
                    TypedExpression::Uint(e) => Ok(UExpression::not(e).into()),
                    e => Err(ErrorInner {
                        pos: Some(pos),

                        message: format!("cannot negate {}", e.get_type()),
                    }),
                }
            }
        }
    }

    fn get_scope(&self, variable_name: &'ast str) -> Option<&'ast ScopedVariable> {
        self.scope.get(&ScopedVariable {
            id: Variable::with_id_and_type(
                crate::typed_absy::Identifier::from(variable_name),
                Type::FieldElement,
            ),
            level: 0,
        })
    }

    fn insert_into_scope(&mut self, v: Variable<'ast>) -> bool {
        self.scope.insert(ScopedVariable {
            id: v,
            level: self.level,
        })
    }

    fn find_function(&self, query: &FunctionQuery<'ast>) -> Option<FunctionKey<'ast>> {
        query.match_funcs(&self.functions)
    }

    fn enter_scope(&mut self) {
        self.level += 1;
    }

    fn exit_scope(&mut self) {
        let current_level = self.level;
        self.scope
            .retain(|ref scoped_variable| scoped_variable.level < current_level);
        self.level -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::absy;
    use crate::typed_absy;
    use num_bigint::BigUint;
    use zokrates_field::Bn128Field;

    const MODULE_ID: &str = "";

    mod constants {
        use super::*;
        use num_bigint::BigUint;
        use std::ops::Add;

        #[test]
        fn field_in_range() {
            let types = HashMap::new();
            let module_id = "".into();

            let expr =
                Expression::FieldConstant(BigUint::from(Bn128Field::max_value().to_biguint()))
                    .mock();
            assert!(Checker::new()
                .check_expression::<Bn128Field>(expr, &module_id, &types)
                .is_ok());
        }

        #[test]
        fn field_overflow() {
            let types = HashMap::new();
            let module_id = "".into();

            let value = Bn128Field::max_value().to_biguint().add(1u32);
            let expr = Expression::FieldConstant(BigUint::from(value)).mock();

            assert!(Checker::new()
                .check_expression::<Bn128Field>(expr, &module_id, &types)
                .is_err());
        }
    }

    mod array {
        use super::*;
        use num_bigint::BigUint;

        #[test]
        fn element_type_mismatch() {
            let types = HashMap::new();
            let module_id = "".into();
            // [3, true]
            let a = Expression::InlineArray(vec![
                Expression::FieldConstant(BigUint::from(3u32)).mock().into(),
                Expression::BooleanConstant(true).mock().into(),
            ])
            .mock();
            assert!(Checker::new()
                .check_expression::<Bn128Field>(a, &module_id, &types)
                .is_err());

            // [[0], [0, 0]]
            let a = Expression::InlineArray(vec![
                Expression::InlineArray(vec![Expression::FieldConstant(BigUint::from(0u32))
                    .mock()
                    .into()])
                .mock()
                .into(),
                Expression::InlineArray(vec![
                    Expression::FieldConstant(BigUint::from(0u32)).mock().into(),
                    Expression::FieldConstant(BigUint::from(0u32)).mock().into(),
                ])
                .mock()
                .into(),
            ])
            .mock();
            assert!(Checker::new()
                .check_expression::<Bn128Field>(a, &module_id, &types)
                .is_err());

            // [[0], true]
            let a = Expression::InlineArray(vec![
                Expression::InlineArray(vec![Expression::FieldConstant(BigUint::from(0u32))
                    .mock()
                    .into()])
                .mock()
                .into(),
                Expression::InlineArray(vec![Expression::BooleanConstant(true).mock().into()])
                    .mock()
                    .into(),
            ])
            .mock();
            assert!(Checker::new()
                .check_expression::<Bn128Field>(a, &module_id, &types)
                .is_err());
        }
    }

    mod symbols {
        use super::*;

        /// Helper function to create ((): return)
        fn function0() -> FunctionNode<'static> {
            let statements: Vec<StatementNode> = vec![Statement::Return(
                ExpressionList {
                    expressions: vec![],
                }
                .mock(),
            )
            .mock()];

            let arguments = vec![];

            let signature = UnresolvedSignature::new();

            Function {
                arguments,
                statements,
                signature,
            }
            .mock()
        }

        /// Helper function to create ((private field a): return)
        fn function1() -> FunctionNode<'static> {
            let statements: Vec<StatementNode> = vec![Statement::Return(
                ExpressionList {
                    expressions: vec![],
                }
                .mock(),
            )
            .mock()];

            let arguments = vec![absy::Parameter {
                id: absy::Variable::new("a", UnresolvedType::FieldElement.mock()).mock(),
                private: true,
            }
            .mock()];

            let signature =
                UnresolvedSignature::new().inputs(vec![UnresolvedType::FieldElement.mock()]);

            Function {
                arguments,
                statements,
                signature,
            }
            .mock()
        }

        fn struct0() -> StructDefinitionNode<'static> {
            StructDefinition { fields: vec![] }.mock()
        }

        fn struct1() -> StructDefinitionNode<'static> {
            StructDefinition {
                fields: vec![StructDefinitionField {
                    id: "foo".into(),
                    ty: UnresolvedType::FieldElement.mock(),
                }
                .mock()],
            }
            .mock()
        }

        #[test]
        fn unifier() {
            // the unifier should only accept either a single type or many functions of different signatures for each symbol

            let mut unifier = SymbolUnifier::default();

            assert!(unifier.insert_type("foo"));
            assert!(!unifier.insert_type("foo"));
            assert!(!unifier.insert_function("foo", Signature::new()));
            assert!(unifier.insert_function("bar", Signature::new()));
            assert!(!unifier.insert_function("bar", Signature::new()));
            assert!(
                unifier.insert_function("bar", Signature::new().inputs(vec![Type::FieldElement]))
            );
            assert!(!unifier.insert_type("bar"));
        }

        #[test]
        fn imported_function() {
            // foo.zok
            // def main():
            // 		return

            // bar.zok
            // from "./foo.zok" import main

            // after semantic check, `bar` should import a checked function

            let foo: Module = Module {
                symbols: vec![SymbolDeclaration {
                    id: "main",
                    symbol: Symbol::HereFunction(function0()),
                }
                .mock()],
                imports: vec![],
            };

            let bar: Module = Module {
                symbols: vec![SymbolDeclaration {
                    id: "main",
                    symbol: Symbol::There(SymbolImport::with_id_in_module("main", "foo").mock()),
                }
                .mock()],
                imports: vec![],
            };

            let mut state = State::<Bn128Field>::new(
                vec![("foo".into(), foo), ("bar".into(), bar)]
                    .into_iter()
                    .collect(),
            );

            let mut checker = Checker::new();

            assert_eq!(checker.check_module(&"bar".into(), &mut state), Ok(()));
            assert_eq!(
                state.typed_modules.get(&PathBuf::from("bar")),
                Some(&TypedModule {
                    functions: vec![(
                        FunctionKey::with_id("main").signature(Signature::new()),
                        TypedFunctionSymbol::There(
                            FunctionKey::with_id("main").signature(Signature::new()),
                            "foo".into()
                        )
                    )]
                    .into_iter()
                    .collect(),
                })
            );
        }

        #[test]
        fn duplicate_function_declaration() {
            // def foo():
            //   return
            // def foo():
            //   return
            //
            // should fail

            let module = Module {
                symbols: vec![
                    SymbolDeclaration {
                        id: "foo",
                        symbol: Symbol::HereFunction(function0()),
                    }
                    .mock(),
                    SymbolDeclaration {
                        id: "foo",
                        symbol: Symbol::HereFunction(function0()),
                    }
                    .mock(),
                ],
                imports: vec![],
            };

            let mut state = State::<Bn128Field>::new(
                vec![(PathBuf::from(MODULE_ID).into(), module)]
                    .into_iter()
                    .collect(),
            );

            let mut checker = Checker::new();
            assert_eq!(
                checker
                    .check_module(&PathBuf::from(MODULE_ID).into(), &mut state)
                    .unwrap_err()[0]
                    .inner
                    .message,
                "foo conflicts with another symbol"
            );
        }

        #[test]
        fn overloaded_function_declaration() {
            // def foo():
            //   return
            // def foo(a):
            //   return
            //
            // should succeed as overloading is allowed

            let module = Module {
                symbols: vec![
                    SymbolDeclaration {
                        id: "foo",
                        symbol: Symbol::HereFunction(function0()),
                    }
                    .mock(),
                    SymbolDeclaration {
                        id: "foo",
                        symbol: Symbol::HereFunction(function1()),
                    }
                    .mock(),
                ],
                imports: vec![],
            };

            let mut state = State::<Bn128Field>::new(
                vec![(PathBuf::from(MODULE_ID), module)]
                    .into_iter()
                    .collect(),
            );

            let mut checker = Checker::new();
            assert_eq!(
                checker.check_module(&PathBuf::from(MODULE_ID), &mut state),
                Ok(())
            );
            assert!(state
                .typed_modules
                .get(&PathBuf::from(MODULE_ID))
                .unwrap()
                .functions
                .contains_key(&FunctionKey::with_id("foo").signature(Signature::new())));
            assert!(state
                .typed_modules
                .get(&PathBuf::from(MODULE_ID))
                .unwrap()
                .functions
                .contains_key(
                    &FunctionKey::with_id("foo")
                        .signature(Signature::new().inputs(vec![Type::FieldElement]))
                ))
        }

        #[test]
        fn duplicate_type_declaration() {
            // struct Foo {}
            // struct Foo { foo: field }
            //
            // should fail

            let module: Module = Module {
                symbols: vec![
                    SymbolDeclaration {
                        id: "foo",
                        symbol: Symbol::HereType(struct0()),
                    }
                    .mock(),
                    SymbolDeclaration {
                        id: "foo",
                        symbol: Symbol::HereType(struct1()),
                    }
                    .mock(),
                ],
                imports: vec![],
            };

            let mut state =
                State::<Bn128Field>::new(vec![("main".into(), module)].into_iter().collect());

            let mut checker = Checker::new();
            assert_eq!(
                checker
                    .check_module(&"main".into(), &mut state)
                    .unwrap_err()[0]
                    .inner
                    .message,
                "foo conflicts with another symbol"
            );
        }

        #[test]
        fn type_function_conflict() {
            // struct foo {}
            // def foo():
            //   return
            //
            // should fail

            let module = Module {
                symbols: vec![
                    SymbolDeclaration {
                        id: "foo",
                        symbol: Symbol::HereFunction(function0()),
                    }
                    .mock(),
                    SymbolDeclaration {
                        id: "foo",
                        symbol: Symbol::HereType(StructDefinition { fields: vec![] }.mock()),
                    }
                    .mock(),
                ],
                imports: vec![],
            };

            let mut state =
                State::<Bn128Field>::new(vec![("main".into(), module)].into_iter().collect());

            let mut checker = Checker::new();
            assert_eq!(
                checker
                    .check_module(&"main".into(), &mut state)
                    .unwrap_err()[0]
                    .inner
                    .message,
                "foo conflicts with another symbol"
            );
        }

        #[test]
        fn type_imported_function_conflict() {
            // import first

            // // bar.code
            // def main(): return
            //
            // // main.code
            // import main from "bar" as foo
            // struct foo {}
            //
            // should fail

            let bar = Module::with_symbols(vec![SymbolDeclaration {
                id: "main",
                symbol: Symbol::HereFunction(function0()),
            }
            .mock()]);

            let main = Module {
                symbols: vec![
                    SymbolDeclaration {
                        id: "foo",
                        symbol: Symbol::There(
                            SymbolImport::with_id_in_module("main", "bar").mock(),
                        ),
                    }
                    .mock(),
                    SymbolDeclaration {
                        id: "foo",
                        symbol: Symbol::HereType(struct0()),
                    }
                    .mock(),
                ],
                imports: vec![],
            };

            let mut state = State::<Bn128Field>::new(
                vec![(PathBuf::from(MODULE_ID), main), ("bar".into(), bar)]
                    .into_iter()
                    .collect(),
            );

            let mut checker = Checker::new();
            assert_eq!(
                checker
                    .check_module(&PathBuf::from(MODULE_ID), &mut state)
                    .unwrap_err()[0]
                    .inner
                    .message,
                "foo conflicts with another symbol"
            );

            // type declaration first

            // // bar.code
            // def main(): return
            //
            // // main.code
            // struct foo {}
            // import main from "bar" as foo
            //
            // should fail

            let bar = Module::with_symbols(vec![SymbolDeclaration {
                id: "main",
                symbol: Symbol::HereFunction(function0()),
            }
            .mock()]);

            let main = Module {
                symbols: vec![
                    SymbolDeclaration {
                        id: "foo",
                        symbol: Symbol::HereType(struct0()),
                    }
                    .mock(),
                    SymbolDeclaration {
                        id: "foo",
                        symbol: Symbol::There(
                            SymbolImport::with_id_in_module("main", "bar").mock(),
                        ),
                    }
                    .mock(),
                ],
                imports: vec![],
            };

            let mut state = State::<Bn128Field>::new(
                vec![(PathBuf::from(MODULE_ID), main), ("bar".into(), bar)]
                    .into_iter()
                    .collect(),
            );

            let mut checker = Checker::new();
            assert_eq!(
                checker
                    .check_module(&PathBuf::from(MODULE_ID), &mut state)
                    .unwrap_err()[0]
                    .inner
                    .message,
                "foo conflicts with another symbol"
            );
        }
    }

    pub fn new_with_args<'ast>(
        scope: HashSet<ScopedVariable<'ast>>,
        level: usize,
        functions: HashSet<FunctionKey<'ast>>,
    ) -> Checker<'ast> {
        Checker {
            scope,
            functions,
            level,
        }
    }

    #[test]
    fn undefined_variable_in_statement() {
        // a = b
        // b undefined
        let statement: StatementNode = Statement::Definition(
            Assignee::Identifier("a").mock(),
            Expression::Identifier("b").mock(),
        )
        .mock();

        let types = HashMap::new();
        let module_id = "".into();

        let mut checker = Checker::new();
        assert_eq!(
            checker.check_statement::<Bn128Field>(statement, &module_id, &types),
            Err(vec![ErrorInner {
                pos: Some((Position::mock(), Position::mock())),
                message: "Identifier \"b\" is undefined".into()
            }])
        );
    }

    #[test]
    fn defined_variable_in_statement() {
        // a = b
        // b defined
        let statement: StatementNode = Statement::Definition(
            Assignee::Identifier("a").mock(),
            Expression::Identifier("b").mock(),
        )
        .mock();

        let types = HashMap::new();
        let module_id = "".into();

        let mut scope = HashSet::new();
        scope.insert(ScopedVariable {
            id: Variable::field_element("a"),
            level: 0,
        });
        scope.insert(ScopedVariable {
            id: Variable::field_element("b"),
            level: 0,
        });
        let mut checker = new_with_args(scope, 1, HashSet::new());
        assert_eq!(
            checker.check_statement::<Bn128Field>(statement, &module_id, &types),
            Ok(TypedStatement::Definition(
                TypedAssignee::Identifier(typed_absy::Variable::field_element("a")),
                FieldElementExpression::Identifier("b".into()).into()
            ))
        );
    }

    #[test]
    fn declared_in_other_function() {
        // def foo():
        //   field a = 1
        //   return
        // def bar():
        //   return a
        // should fail
        let foo_args = vec![];
        let foo_statements = vec![
            Statement::Declaration(
                absy::Variable::new("a", UnresolvedType::FieldElement.mock()).mock(),
            )
            .mock(),
            Statement::Definition(
                Assignee::Identifier("a").mock(),
                Expression::FieldConstant(BigUint::from(1u32)).mock(),
            )
            .mock(),
            Statement::Return(
                ExpressionList {
                    expressions: vec![],
                }
                .mock(),
            )
            .mock(),
        ];
        let foo = Function {
            arguments: foo_args,
            statements: foo_statements,
            signature: UnresolvedSignature::new(),
        }
        .mock();

        let bar_args = vec![];
        let bar_statements = vec![Statement::Return(
            ExpressionList {
                expressions: vec![Expression::Identifier("a").mock()],
            }
            .mock(),
        )
        .mock()];

        let bar = Function {
            arguments: bar_args,
            statements: bar_statements,
            signature: UnresolvedSignature {
                inputs: vec![],
                outputs: vec![UnresolvedType::FieldElement.mock()],
            },
        }
        .mock();

        let symbols = vec![
            SymbolDeclaration {
                id: "foo",
                symbol: Symbol::HereFunction(foo),
            }
            .mock(),
            SymbolDeclaration {
                id: "bar",
                symbol: Symbol::HereFunction(bar),
            }
            .mock(),
        ];
        let module = Module {
            symbols,
            imports: vec![],
        };

        let mut state =
            State::<Bn128Field>::new(vec![("main".into(), module)].into_iter().collect());

        let mut checker = Checker::new();
        assert_eq!(
            checker.check_module(&"main".into(), &mut state),
            Err(vec![Error {
                inner: ErrorInner {
                    pos: Some((Position::mock(), Position::mock())),
                    message: "Identifier \"a\" is undefined".into()
                },
                module_id: "main".into()
            }])
        );
    }

    #[test]
    fn declared_in_two_scopes() {
        // def foo():
        //   a = 1
        //   return
        // def bar():
        //   a = 2
        //   return
        // def main():
        //   return 1
        // should pass
        let foo_args = vec![];
        let foo_statements = vec![
            Statement::Declaration(
                absy::Variable::new("a", UnresolvedType::FieldElement.mock()).mock(),
            )
            .mock(),
            Statement::Definition(
                Assignee::Identifier("a").mock(),
                Expression::FieldConstant(BigUint::from(1u32)).mock(),
            )
            .mock(),
            Statement::Return(
                ExpressionList {
                    expressions: vec![],
                }
                .mock(),
            )
            .mock(),
        ];

        let foo = Function {
            arguments: foo_args,
            statements: foo_statements,
            signature: UnresolvedSignature::new(),
        }
        .mock();

        let bar_args = vec![];
        let bar_statements = vec![
            Statement::Declaration(
                absy::Variable::new("a", UnresolvedType::FieldElement.mock()).mock(),
            )
            .mock(),
            Statement::Definition(
                Assignee::Identifier("a").mock(),
                Expression::FieldConstant(BigUint::from(2u32)).mock(),
            )
            .mock(),
            Statement::Return(
                ExpressionList {
                    expressions: vec![],
                }
                .mock(),
            )
            .mock(),
        ];
        let bar = Function {
            arguments: bar_args,
            statements: bar_statements,
            signature: UnresolvedSignature::new(),
        }
        .mock();

        let main_args = vec![];
        let main_statements = vec![Statement::Return(
            ExpressionList {
                expressions: vec![Expression::FieldConstant(BigUint::from(1u32)).mock()],
            }
            .mock(),
        )
        .mock()];

        let main = Function {
            arguments: main_args,
            statements: main_statements,
            signature: UnresolvedSignature {
                inputs: vec![],
                outputs: vec![UnresolvedType::FieldElement.mock()],
            },
        }
        .mock();

        let symbols = vec![
            SymbolDeclaration {
                id: "foo",
                symbol: Symbol::HereFunction(foo),
            }
            .mock(),
            SymbolDeclaration {
                id: "bar",
                symbol: Symbol::HereFunction(bar),
            }
            .mock(),
            SymbolDeclaration {
                id: "main",
                symbol: Symbol::HereFunction(main),
            }
            .mock(),
        ];
        let module = Module {
            symbols,
            imports: vec![],
        };

        let mut state =
            State::<Bn128Field>::new(vec![("main".into(), module)].into_iter().collect());

        let mut checker = Checker::new();
        assert!(checker.check_module(&"main".into(), &mut state).is_ok());
    }

    #[test]
    fn for_index_after_end() {
        // def foo():
        //   for field i in 0..10 do
        //   endfor
        //   return i
        // should fail
        let foo_statements = vec![
            Statement::For(
                absy::Variable::new("i", UnresolvedType::FieldElement.mock()).mock(),
                Expression::FieldConstant(BigUint::from(0u32)).mock(),
                Expression::FieldConstant(BigUint::from(10u32)).mock(),
                vec![],
            )
            .mock(),
            Statement::Return(
                ExpressionList {
                    expressions: vec![Expression::Identifier("i").mock()],
                }
                .mock(),
            )
            .mock(),
        ];
        let foo = Function {
            arguments: vec![],
            statements: foo_statements,
            signature: UnresolvedSignature {
                inputs: vec![],
                outputs: vec![UnresolvedType::FieldElement.mock()],
            },
        }
        .mock();

        let types = HashMap::new();
        let module_id = "".into();

        let mut checker = Checker::new();
        assert_eq!(
            checker.check_function::<Bn128Field>(foo, &module_id, &types),
            Err(vec![ErrorInner {
                pos: Some((Position::mock(), Position::mock())),
                message: "Identifier \"i\" is undefined".into()
            }])
        );
    }

    #[test]
    fn for_index_in_for() {
        // def foo():
        //   for i in 0..10 do
        //     a = i
        //   endfor
        //   return
        // should pass

        let for_statements = vec![
            Statement::Declaration(
                absy::Variable::new("a", UnresolvedType::FieldElement.mock()).mock(),
            )
            .mock(),
            Statement::Definition(
                Assignee::Identifier("a").mock(),
                Expression::Identifier("i").mock(),
            )
            .mock(),
        ];

        let foo_statements = vec![
            Statement::For(
                absy::Variable::new("i", UnresolvedType::FieldElement.mock()).mock(),
                Expression::FieldConstant(BigUint::from(0u32)).mock(),
                Expression::FieldConstant(BigUint::from(10u32)).mock(),
                for_statements,
            )
            .mock(),
            Statement::Return(
                ExpressionList {
                    expressions: vec![],
                }
                .mock(),
            )
            .mock(),
        ];

        let for_statements_checked = vec![
            TypedStatement::Declaration(typed_absy::Variable::field_element("a")),
            TypedStatement::Definition(
                TypedAssignee::Identifier(typed_absy::Variable::field_element("a")),
                FieldElementExpression::Identifier("i".into()).into(),
            ),
        ];

        let foo_statements_checked = vec![
            TypedStatement::For(
                typed_absy::Variable::field_element("i"),
                FieldElementExpression::Number(Bn128Field::from(0u32)),
                FieldElementExpression::Number(Bn128Field::from(10u32)),
                for_statements_checked,
            ),
            TypedStatement::Return(vec![]),
        ];

        let foo = Function {
            arguments: vec![],
            statements: foo_statements,
            signature: UnresolvedSignature::new(),
        }
        .mock();

        let foo_checked = TypedFunction {
            arguments: vec![],
            statements: foo_statements_checked,
            signature: Signature::new(),
        };

        let types = HashMap::new();
        let module_id = "".into();

        let mut checker = Checker::new();
        assert_eq!(
            checker.check_function::<Bn128Field>(foo, &module_id, &types),
            Ok(foo_checked)
        );
    }

    #[test]
    fn arity_mismatch() {
        // def foo():
        //   return 1, 2
        // def bar():
        //   field a = foo()
        // should fail
        let bar_statements: Vec<StatementNode> = vec![
            Statement::Declaration(
                absy::Variable::new("a", UnresolvedType::FieldElement.mock()).mock(),
            )
            .mock(),
            Statement::MultipleDefinition(
                vec![Assignee::Identifier("a").mock()],
                Expression::FunctionCall("foo", vec![]).mock(),
            )
            .mock(),
            Statement::Return(
                ExpressionList {
                    expressions: vec![],
                }
                .mock(),
            )
            .mock(),
        ];

        let foo = FunctionKey {
            id: "foo",
            signature: Signature {
                inputs: vec![],
                outputs: vec![Type::FieldElement, Type::FieldElement],
            },
        };

        let functions = vec![foo].into_iter().collect();

        let bar = Function {
            arguments: vec![],
            statements: bar_statements,
            signature: UnresolvedSignature::new(),
        }
        .mock();

        let types = HashMap::new();
        let module_id = "".into();

        let mut checker = new_with_args(HashSet::new(), 0, functions);
        assert_eq!(
            checker.check_function::<Bn128Field>(bar, &module_id, &types),
            Err(vec![ErrorInner {
                pos: Some((Position::mock(), Position::mock())),
                message:
                    "Function definition for function foo with signature () -> field not found."
                        .into()
            }])
        );
    }

    #[test]
    fn multi_return_outside_multidef() {
        // def foo() -> (field, field):
        //   return 1, 2
        // def bar():
        //   2 == foo()
        //   return
        // should fail
        let bar_statements: Vec<StatementNode> = vec![
            Statement::Assertion(
                Expression::Eq(
                    box Expression::FieldConstant(BigUint::from(2u32)).mock(),
                    box Expression::FunctionCall("foo", vec![]).mock(),
                )
                .mock(),
            )
            .mock(),
            Statement::Return(
                ExpressionList {
                    expressions: vec![],
                }
                .mock(),
            )
            .mock(),
        ];

        let foo = FunctionKey {
            id: "foo",
            signature: Signature {
                inputs: vec![],
                outputs: vec![Type::FieldElement, Type::FieldElement],
            },
        };

        let functions = vec![foo].into_iter().collect();

        let bar = Function {
            arguments: vec![],
            statements: bar_statements,
            signature: UnresolvedSignature {
                inputs: vec![],
                outputs: vec![],
            },
        }
        .mock();

        let types = HashMap::new();
        let module_id = "".into();

        let mut checker = new_with_args(HashSet::new(), 0, functions);
        assert_eq!(
            checker.check_function::<Bn128Field>(bar, &module_id, &types),
            Err(vec![ErrorInner {
                pos: Some((Position::mock(), Position::mock())),
                message: "Function definition for function foo with signature () -> _ not found."
                    .into()
            }])
        );
    }

    #[test]
    fn function_undefined_in_multidef() {
        // def bar():
        //   field a = foo()
        //   return
        // should fail
        let bar_statements: Vec<StatementNode> = vec![
            Statement::Declaration(
                absy::Variable::new("a", UnresolvedType::FieldElement.mock()).mock(),
            )
            .mock(),
            Statement::MultipleDefinition(
                vec![Assignee::Identifier("a").mock()],
                Expression::FunctionCall("foo", vec![]).mock(),
            )
            .mock(),
            Statement::Return(
                ExpressionList {
                    expressions: vec![],
                }
                .mock(),
            )
            .mock(),
        ];

        let bar = Function {
            arguments: vec![],
            statements: bar_statements,
            signature: UnresolvedSignature::new(),
        }
        .mock();

        let types = HashMap::new();
        let module_id = "".into();

        let mut checker = new_with_args(HashSet::new(), 0, HashSet::new());
        assert_eq!(
            checker.check_function::<Bn128Field>(bar, &module_id, &types),
            Err(vec![ErrorInner {
                pos: Some((Position::mock(), Position::mock())),

                message:
                    "Function definition for function foo with signature () -> field not found."
                        .into()
            }])
        );
    }

    #[test]
    fn undefined_variable_in_multireturn_call() {
        // def foo(x):
        // 	return 1, 2
        // def main():
        // 	a, b = foo(x)
        // 	return 1
        // should fail

        let foo_statements: Vec<StatementNode> = vec![Statement::Return(
            ExpressionList {
                expressions: vec![
                    Expression::FieldConstant(BigUint::from(1u32)).mock(),
                    Expression::FieldConstant(BigUint::from(2u32)).mock(),
                ],
            }
            .mock(),
        )
        .mock()];

        let foo = Function {
            arguments: vec![crate::absy::Parameter {
                id: absy::Variable::new("x", UnresolvedType::FieldElement.mock()).mock(),
                private: false,
            }
            .mock()],
            statements: foo_statements,
            signature: UnresolvedSignature {
                inputs: vec![UnresolvedType::FieldElement.mock()],
                outputs: vec![
                    UnresolvedType::FieldElement.mock(),
                    UnresolvedType::FieldElement.mock(),
                ],
            },
        }
        .mock();

        let main_statements: Vec<StatementNode> = vec![
            Statement::Declaration(
                absy::Variable::new("a", UnresolvedType::FieldElement.mock()).mock(),
            )
            .mock(),
            Statement::Declaration(
                absy::Variable::new("b", UnresolvedType::FieldElement.mock()).mock(),
            )
            .mock(),
            Statement::MultipleDefinition(
                vec![
                    Assignee::Identifier("a").mock(),
                    Assignee::Identifier("b").mock(),
                ],
                Expression::FunctionCall("foo", vec![Expression::Identifier("x").mock()]).mock(),
            )
            .mock(),
            Statement::Return(
                ExpressionList {
                    expressions: vec![Expression::FieldConstant(BigUint::from(1u32)).mock()],
                }
                .mock(),
            )
            .mock(),
        ];

        let main = Function {
            arguments: vec![],
            statements: main_statements,
            signature: UnresolvedSignature {
                inputs: vec![],
                outputs: vec![UnresolvedType::FieldElement.mock()],
            },
        }
        .mock();

        let module = Module {
            symbols: vec![
                SymbolDeclaration {
                    id: "foo",
                    symbol: Symbol::HereFunction(foo),
                }
                .mock(),
                SymbolDeclaration {
                    id: "main",
                    symbol: Symbol::HereFunction(main),
                }
                .mock(),
            ],
            imports: vec![],
        };

        let mut state =
            State::<Bn128Field>::new(vec![("main".into(), module)].into_iter().collect());

        let mut checker = new_with_args(HashSet::new(), 0, HashSet::new());
        assert_eq!(
            checker.check_module(&"main".into(), &mut state),
            Err(vec![Error {
                inner: ErrorInner {
                    pos: Some((Position::mock(), Position::mock())),
                    message: "Identifier \"x\" is undefined".into()
                },
                module_id: "main".into()
            }])
        );
    }

    #[test]
    fn undeclared_variables() {
        // def foo() -> (field, field):
        //  return 1, 2
        // def main():
        //  a, b = foo()
        //  return 1
        // should fail

        let foo_statements: Vec<StatementNode> = vec![Statement::Return(
            ExpressionList {
                expressions: vec![
                    Expression::FieldConstant(BigUint::from(1u32)).mock(),
                    Expression::FieldConstant(BigUint::from(2u32)).mock(),
                ],
            }
            .mock(),
        )
        .mock()];

        let foo = Function {
            arguments: vec![],
            statements: foo_statements,
            signature: UnresolvedSignature {
                inputs: vec![],
                outputs: vec![
                    UnresolvedType::FieldElement.mock(),
                    UnresolvedType::FieldElement.mock(),
                ],
            },
        }
        .mock();

        let main_statements: Vec<StatementNode> = vec![
            Statement::MultipleDefinition(
                vec![
                    Assignee::Identifier("a").mock(),
                    Assignee::Identifier("b").mock(),
                ],
                Expression::FunctionCall("foo", vec![]).mock(),
            )
            .mock(),
            Statement::Return(
                ExpressionList {
                    expressions: vec![],
                }
                .mock(),
            )
            .mock(),
        ];

        let main = Function {
            arguments: vec![],
            statements: main_statements,
            signature: UnresolvedSignature {
                inputs: vec![],
                outputs: vec![],
            },
        }
        .mock();

        let module = Module {
            symbols: vec![
                SymbolDeclaration {
                    id: "foo",
                    symbol: Symbol::HereFunction(foo),
                }
                .mock(),
                SymbolDeclaration {
                    id: "main",
                    symbol: Symbol::HereFunction(main),
                }
                .mock(),
            ],
            imports: vec![],
        };

        let mut state =
            State::<Bn128Field>::new(vec![("main".into(), module)].into_iter().collect());

        let mut checker = new_with_args(HashSet::new(), 0, HashSet::new());
        assert_eq!(
            checker.check_module(&"main".into(), &mut state),
            Err(vec![
                Error {
                    inner: ErrorInner {
                        pos: Some((Position::mock(), Position::mock())),
                        message: "Variable `a` is undeclared".into()
                    },
                    module_id: "main".into()
                },
                Error {
                    inner: ErrorInner {
                        pos: Some((Position::mock(), Position::mock())),
                        message: "Variable `b` is undeclared".into()
                    },
                    module_id: "main".into()
                }
            ])
        );
    }

    #[test]
    fn assign_to_select() {
        // def foo() -> field:
        //  return 1
        // def main():
        //  field[1] a = [0]
        //  a[0] = foo()
        //  return
        // should succeed

        let foo_statements: Vec<StatementNode> = vec![Statement::Return(
            ExpressionList {
                expressions: vec![Expression::FieldConstant(BigUint::from(1u32)).mock()],
            }
            .mock(),
        )
        .mock()];

        let foo = Function {
            arguments: vec![],
            statements: foo_statements,
            signature: UnresolvedSignature {
                inputs: vec![],
                outputs: vec![UnresolvedType::FieldElement.mock()],
            },
        }
        .mock();

        let main_statements: Vec<StatementNode> = vec![
            Statement::Declaration(
                absy::Variable::new(
                    "a",
                    UnresolvedType::array(UnresolvedType::FieldElement.mock(), 1).mock(),
                )
                .mock(),
            )
            .mock(),
            Statement::Definition(
                Assignee::Identifier("a".into()).mock(),
                Expression::InlineArray(vec![absy::SpreadOrExpression::Expression(
                    Expression::FieldConstant(BigUint::from(0u32)).mock(),
                )])
                .mock(),
            )
            .mock(),
            Statement::MultipleDefinition(
                vec![Assignee::Select(
                    box Assignee::Identifier("a").mock(),
                    box RangeOrExpression::Expression(
                        absy::Expression::FieldConstant(BigUint::from(0u32)).mock(),
                    ),
                )
                .mock()],
                Expression::FunctionCall("foo", vec![]).mock(),
            )
            .mock(),
            Statement::Return(
                ExpressionList {
                    expressions: vec![],
                }
                .mock(),
            )
            .mock(),
        ];

        let main = Function {
            arguments: vec![],
            statements: main_statements,
            signature: UnresolvedSignature {
                inputs: vec![],
                outputs: vec![],
            },
        }
        .mock();

        let module = Module {
            symbols: vec![
                SymbolDeclaration {
                    id: "foo",
                    symbol: Symbol::HereFunction(foo),
                }
                .mock(),
                SymbolDeclaration {
                    id: "main",
                    symbol: Symbol::HereFunction(main),
                }
                .mock(),
            ],
            imports: vec![],
        };

        let mut state =
            State::<Bn128Field>::new(vec![("main".into(), module)].into_iter().collect());

        let mut checker = new_with_args(HashSet::new(), 0, HashSet::new());
        assert!(checker.check_module(&"main".into(), &mut state).is_ok());
    }

    #[test]
    fn function_undefined() {
        // def bar():
        //   1 == foo()
        //   return
        // should fail
        let bar_statements: Vec<StatementNode> = vec![
            Statement::Assertion(
                Expression::Eq(
                    box Expression::FieldConstant(BigUint::from(1u32)).mock(),
                    box Expression::FunctionCall("foo", vec![]).mock(),
                )
                .mock(),
            )
            .mock(),
            Statement::Return(
                ExpressionList {
                    expressions: vec![],
                }
                .mock(),
            )
            .mock(),
        ];

        let bar = Function {
            arguments: vec![],
            statements: bar_statements,
            signature: UnresolvedSignature::new(),
        }
        .mock();

        let types = HashMap::new();
        let module_id = "".into();

        let mut checker = new_with_args(HashSet::new(), 0, HashSet::new());
        assert_eq!(
            checker.check_function::<Bn128Field>(bar, &module_id, &types),
            Err(vec![ErrorInner {
                pos: Some((Position::mock(), Position::mock())),

                message: "Function definition for function foo with signature () -> _ not found."
                    .into()
            }])
        );
    }

    #[test]
    fn return_undefined() {
        // def bar():
        //   return a, b
        // should fail
        let bar_statements: Vec<StatementNode> = vec![Statement::Return(
            ExpressionList {
                expressions: vec![
                    Expression::Identifier("a").mock(),
                    Expression::Identifier("b").mock(),
                ],
            }
            .mock(),
        )
        .mock()];

        let bar = Function {
            arguments: vec![],
            statements: bar_statements,
            signature: UnresolvedSignature {
                inputs: vec![],
                outputs: vec![
                    UnresolvedType::FieldElement.mock(),
                    UnresolvedType::FieldElement.mock(),
                ],
            },
        }
        .mock();

        let types = HashMap::new();
        let module_id = "".into();

        let mut checker = new_with_args(HashSet::new(), 0, HashSet::new());
        assert_eq!(
            checker.check_function::<Bn128Field>(bar, &module_id, &types),
            Err(vec![ErrorInner {
                pos: Some((Position::mock(), Position::mock())),
                message: "Identifier \"a\" is undefined".into()
            }])
        );
    }

    #[test]
    fn multi_def() {
        // def foo():
        //   return 1, 2
        // def bar():
        //   field a, field b = foo()
        //   return a + b
        //
        // should pass
        let bar_statements: Vec<StatementNode> = vec![
            Statement::Declaration(
                absy::Variable::new("a", UnresolvedType::FieldElement.mock()).mock(),
            )
            .mock(),
            Statement::Declaration(
                absy::Variable::new("b", UnresolvedType::FieldElement.mock()).mock(),
            )
            .mock(),
            Statement::MultipleDefinition(
                vec![
                    Assignee::Identifier("a").mock(),
                    Assignee::Identifier("b").mock(),
                ],
                Expression::FunctionCall("foo", vec![]).mock(),
            )
            .mock(),
            Statement::Return(
                ExpressionList {
                    expressions: vec![Expression::Add(
                        box Expression::Identifier("a").mock(),
                        box Expression::Identifier("b").mock(),
                    )
                    .mock()],
                }
                .mock(),
            )
            .mock(),
        ];

        let bar_statements_checked: Vec<TypedStatement<Bn128Field>> = vec![
            TypedStatement::Declaration(typed_absy::Variable::field_element("a")),
            TypedStatement::Declaration(typed_absy::Variable::field_element("b")),
            TypedStatement::MultipleDefinition(
                vec![
                    typed_absy::Variable::field_element("a").into(),
                    typed_absy::Variable::field_element("b").into(),
                ],
                TypedExpressionList::FunctionCall(
                    FunctionKey::with_id("foo").signature(
                        Signature::new().outputs(vec![Type::FieldElement, Type::FieldElement]),
                    ),
                    vec![],
                    vec![Type::FieldElement, Type::FieldElement],
                ),
            ),
            TypedStatement::Return(vec![FieldElementExpression::Add(
                box FieldElementExpression::Identifier("a".into()),
                box FieldElementExpression::Identifier("b".into()),
            )
            .into()]),
        ];

        let foo = FunctionKey {
            id: "foo",
            signature: Signature {
                inputs: vec![],
                outputs: vec![Type::FieldElement, Type::FieldElement],
            },
        };

        let mut functions = HashSet::new();
        functions.insert(foo);

        let bar = Function {
            arguments: vec![],
            statements: bar_statements,
            signature: UnresolvedSignature {
                inputs: vec![],
                outputs: vec![UnresolvedType::FieldElement.mock()],
            },
        }
        .mock();

        let bar_checked = TypedFunction {
            arguments: vec![],
            statements: bar_statements_checked,
            signature: Signature {
                inputs: vec![],
                outputs: vec![Type::FieldElement],
            },
        };

        let types = HashMap::new();
        let module_id = "".into();

        let mut checker = new_with_args(HashSet::new(), 0, functions);
        assert_eq!(
            checker.check_function(bar, &module_id, &types),
            Ok(bar_checked)
        );
    }

    #[test]
    fn duplicate_main_function() {
        // def main(a):
        //   return 1
        // def main():
        //   return 1
        //
        // should fail
        let main1_statements: Vec<StatementNode> = vec![Statement::Return(
            ExpressionList {
                expressions: vec![Expression::FieldConstant(BigUint::from(1u32)).mock()],
            }
            .mock(),
        )
        .mock()];

        let main1_arguments = vec![crate::absy::Parameter {
            id: absy::Variable::new("a", UnresolvedType::FieldElement.mock()).mock(),
            private: false,
        }
        .mock()];

        let main2_statements: Vec<StatementNode> = vec![Statement::Return(
            ExpressionList {
                expressions: vec![Expression::FieldConstant(BigUint::from(1u32)).mock()],
            }
            .mock(),
        )
        .mock()];

        let main2_arguments = vec![];

        let main1 = Function {
            arguments: main1_arguments,
            statements: main1_statements,
            signature: UnresolvedSignature {
                inputs: vec![UnresolvedType::FieldElement.mock()],
                outputs: vec![UnresolvedType::FieldElement.mock()],
            },
        }
        .mock();

        let main2 = Function {
            arguments: main2_arguments,
            statements: main2_statements,
            signature: UnresolvedSignature {
                inputs: vec![],
                outputs: vec![UnresolvedType::FieldElement.mock()],
            },
        }
        .mock();

        let symbols = vec![
            SymbolDeclaration {
                id: "main",
                symbol: Symbol::HereFunction(main1),
            }
            .mock(),
            SymbolDeclaration {
                id: "main",
                symbol: Symbol::HereFunction(main2),
            }
            .mock(),
        ];

        let main_module = Module {
            symbols,
            imports: vec![],
        };

        let program = Program {
            modules: vec![("main".into(), main_module)].into_iter().collect(),
            main: "main".into(),
        };

        let mut checker = Checker::new();
        assert_eq!(
            checker.check_program::<Bn128Field>(program),
            Err(vec![Error {
                inner: ErrorInner {
                    pos: None,
                    message: "Only one main function allowed, found 2".into()
                },
                module_id: "main".into()
            }])
        );
    }

    #[test]
    fn shadowing_with_same_type() {
        //   field a
        //	 field a
        //
        // should fail

        let types = HashMap::new();
        let module_id = "".into();
        let mut checker = Checker::new();
        let _: Result<TypedStatement<Bn128Field>, Vec<ErrorInner>> = checker.check_statement(
            Statement::Declaration(
                absy::Variable::new("a", UnresolvedType::FieldElement.mock()).mock(),
            )
            .mock(),
            &module_id,
            &types,
        );
        let s2_checked: Result<TypedStatement<Bn128Field>, Vec<ErrorInner>> = checker
            .check_statement(
                Statement::Declaration(
                    absy::Variable::new("a", UnresolvedType::FieldElement.mock()).mock(),
                )
                .mock(),
                &module_id,
                &types,
            );
        assert_eq!(
            s2_checked,
            Err(vec![ErrorInner {
                pos: Some((Position::mock(), Position::mock())),
                message: "Duplicate declaration for variable named a".into()
            }])
        );
    }

    #[test]
    fn shadowing_with_different_type() {
        //   field a
        //	 bool a
        //
        // should fail

        let types = HashMap::new();
        let module_id = "".into();

        let mut checker = Checker::new();
        let _: Result<TypedStatement<Bn128Field>, Vec<ErrorInner>> = checker.check_statement(
            Statement::Declaration(
                absy::Variable::new("a", UnresolvedType::FieldElement.mock()).mock(),
            )
            .mock(),
            &module_id,
            &types,
        );
        let s2_checked: Result<TypedStatement<Bn128Field>, Vec<ErrorInner>> = checker
            .check_statement(
                Statement::Declaration(
                    absy::Variable::new("a", UnresolvedType::Boolean.mock()).mock(),
                )
                .mock(),
                &module_id,
                &types,
            );
        assert_eq!(
            s2_checked,
            Err(vec![ErrorInner {
                pos: Some((Position::mock(), Position::mock())),
                message: "Duplicate declaration for variable named a".into()
            }])
        );
    }

    mod structs {
        use super::*;

        /// solver function to create a module at location "" with a single symbol `Foo { foo: field }`
        fn create_module_with_foo(
            s: StructDefinition<'static>,
        ) -> (Checker<'static>, State<'static, Bn128Field>) {
            let module_id: PathBuf = "".into();

            let module: Module = Module {
                imports: vec![],
                symbols: vec![SymbolDeclaration {
                    id: "Foo",
                    symbol: Symbol::HereType(s.mock()),
                }
                .mock()],
            };

            let mut state =
                State::<Bn128Field>::new(vec![(module_id.clone(), module)].into_iter().collect());

            let mut checker = Checker::new();

            checker.check_module(&module_id, &mut state).unwrap();

            (checker, state)
        }

        /// tests about declaring a type
        mod declaration {
            use super::*;

            #[test]
            fn empty_def() {
                // an empty struct should be allowed to be defined
                let module_id = "".into();
                let types = HashMap::new();
                let declaration = StructDefinition { fields: vec![] }.mock();

                let expected_type = Type::Struct(StructType::new("".into(), "Foo".into(), vec![]));

                assert_eq!(
                    Checker::new().check_struct_type_declaration(
                        "Foo".into(),
                        declaration,
                        &module_id,
                        &types
                    ),
                    Ok(expected_type)
                );
            }

            #[test]
            fn valid_def() {
                // a valid struct should be allowed to be defined
                let module_id = "".into();
                let types = HashMap::new();
                let declaration = StructDefinition {
                    fields: vec![
                        StructDefinitionField {
                            id: "foo",
                            ty: UnresolvedType::FieldElement.mock(),
                        }
                        .mock(),
                        StructDefinitionField {
                            id: "bar",
                            ty: UnresolvedType::Boolean.mock(),
                        }
                        .mock(),
                    ],
                }
                .mock();

                let expected_type = Type::Struct(StructType::new(
                    "".into(),
                    "Foo".into(),
                    vec![
                        StructMember::new("foo".into(), Type::FieldElement),
                        StructMember::new("bar".into(), Type::Boolean),
                    ],
                ));

                assert_eq!(
                    Checker::new().check_struct_type_declaration(
                        "Foo".into(),
                        declaration,
                        &module_id,
                        &types
                    ),
                    Ok(expected_type)
                );
            }

            #[test]
            fn preserve_order() {
                // two structs with inverted members are not equal
                let module_id = "".into();
                let types = HashMap::new();

                let declaration0 = StructDefinition {
                    fields: vec![
                        StructDefinitionField {
                            id: "foo",
                            ty: UnresolvedType::FieldElement.mock(),
                        }
                        .mock(),
                        StructDefinitionField {
                            id: "bar",
                            ty: UnresolvedType::Boolean.mock(),
                        }
                        .mock(),
                    ],
                }
                .mock();

                let declaration1 = StructDefinition {
                    fields: vec![
                        StructDefinitionField {
                            id: "bar",
                            ty: UnresolvedType::Boolean.mock(),
                        }
                        .mock(),
                        StructDefinitionField {
                            id: "foo",
                            ty: UnresolvedType::FieldElement.mock(),
                        }
                        .mock(),
                    ],
                }
                .mock();

                assert_ne!(
                    Checker::new().check_struct_type_declaration(
                        "Foo".into(),
                        declaration0,
                        &module_id,
                        &types
                    ),
                    Checker::new().check_struct_type_declaration(
                        "Foo".into(),
                        declaration1,
                        &module_id,
                        &types
                    )
                );
            }

            #[test]
            fn duplicate_member_def() {
                // definition of a struct with a duplicate member should be rejected
                let module_id = "".into();
                let types = HashMap::new();

                let declaration = StructDefinition {
                    fields: vec![
                        StructDefinitionField {
                            id: "foo",
                            ty: UnresolvedType::FieldElement.mock(),
                        }
                        .mock(),
                        StructDefinitionField {
                            id: "foo",
                            ty: UnresolvedType::Boolean.mock(),
                        }
                        .mock(),
                    ],
                }
                .mock();

                assert_eq!(
                    Checker::new()
                        .check_struct_type_declaration(
                            "Foo".into(),
                            declaration,
                            &module_id,
                            &types
                        )
                        .unwrap_err()[0]
                        .message,
                    "Duplicate key foo in struct definition"
                );
            }

            #[test]
            fn recursive() {
                // a struct wrapping another struct should be allowed to be defined

                // struct Foo = { foo: field }
                // struct Bar = { foo: Foo }

                let module_id: PathBuf = "".into();

                let module: Module = Module {
                    imports: vec![],
                    symbols: vec![
                        SymbolDeclaration {
                            id: "Foo",
                            symbol: Symbol::HereType(
                                StructDefinition {
                                    fields: vec![StructDefinitionField {
                                        id: "foo",
                                        ty: UnresolvedType::FieldElement.mock(),
                                    }
                                    .mock()],
                                }
                                .mock(),
                            ),
                        }
                        .mock(),
                        SymbolDeclaration {
                            id: "Bar",
                            symbol: Symbol::HereType(
                                StructDefinition {
                                    fields: vec![StructDefinitionField {
                                        id: "foo",
                                        ty: UnresolvedType::User("Foo".into()).mock(),
                                    }
                                    .mock()],
                                }
                                .mock(),
                            ),
                        }
                        .mock(),
                    ],
                };

                let mut state = State::<Bn128Field>::new(
                    vec![(module_id.clone(), module)].into_iter().collect(),
                );

                assert!(Checker::new().check_module(&module_id, &mut state).is_ok());
                assert_eq!(
                    state
                        .types
                        .get(&module_id)
                        .unwrap()
                        .get(&"Bar".to_string())
                        .unwrap(),
                    &Type::Struct(StructType::new(
                        module_id.clone(),
                        "Bar".into(),
                        vec![StructMember::new(
                            "foo".into(),
                            Type::Struct(StructType::new(
                                module_id,
                                "Foo".into(),
                                vec![StructMember::new("foo".into(), Type::FieldElement)]
                            ))
                        )]
                    ))
                );
            }

            #[test]
            fn recursive_undefined() {
                // a struct wrapping an undefined struct should be rejected

                // struct Bar = { foo: Foo }

                let module_id: PathBuf = "".into();

                let module: Module = Module {
                    imports: vec![],
                    symbols: vec![SymbolDeclaration {
                        id: "Bar",
                        symbol: Symbol::HereType(
                            StructDefinition {
                                fields: vec![StructDefinitionField {
                                    id: "foo",
                                    ty: UnresolvedType::User("Foo".into()).mock(),
                                }
                                .mock()],
                            }
                            .mock(),
                        ),
                    }
                    .mock()],
                };

                let mut state = State::<Bn128Field>::new(
                    vec![(module_id.clone(), module)].into_iter().collect(),
                );

                assert!(Checker::new().check_module(&module_id, &mut state).is_err());
            }

            #[test]
            fn self_referential() {
                // a struct wrapping itself should be rejected

                // struct Foo = { foo: Foo }

                let module_id: PathBuf = "".into();

                let module: Module = Module {
                    imports: vec![],
                    symbols: vec![SymbolDeclaration {
                        id: "Foo",
                        symbol: Symbol::HereType(
                            StructDefinition {
                                fields: vec![StructDefinitionField {
                                    id: "foo",
                                    ty: UnresolvedType::User("Foo".into()).mock(),
                                }
                                .mock()],
                            }
                            .mock(),
                        ),
                    }
                    .mock()],
                };

                let mut state = State::<Bn128Field>::new(
                    vec![(module_id.clone(), module)].into_iter().collect(),
                );

                assert!(Checker::new().check_module(&module_id, &mut state).is_err());
            }

            #[test]
            fn cyclic() {
                // A wrapping B wrapping A should be rejected

                // struct Foo = { bar: Bar }
                // struct Bar = { foo: Foo }

                let module_id: PathBuf = "".into();

                let module: Module = Module {
                    imports: vec![],
                    symbols: vec![
                        SymbolDeclaration {
                            id: "Foo",
                            symbol: Symbol::HereType(
                                StructDefinition {
                                    fields: vec![StructDefinitionField {
                                        id: "bar",
                                        ty: UnresolvedType::User("Bar".into()).mock(),
                                    }
                                    .mock()],
                                }
                                .mock(),
                            ),
                        }
                        .mock(),
                        SymbolDeclaration {
                            id: "Bar",
                            symbol: Symbol::HereType(
                                StructDefinition {
                                    fields: vec![StructDefinitionField {
                                        id: "foo",
                                        ty: UnresolvedType::User("Foo".into()).mock(),
                                    }
                                    .mock()],
                                }
                                .mock(),
                            ),
                        }
                        .mock(),
                    ],
                };

                let mut state = State::<Bn128Field>::new(
                    vec![(module_id.clone(), module)].into_iter().collect(),
                );

                assert!(Checker::new().check_module(&module_id, &mut state).is_err());
            }
        }

        /// tests about using the defined type identifier
        mod usage {
            use super::*;

            #[test]
            fn ty() {
                // a defined type can be checked
                // Foo { foo: field }
                // Foo

                // an undefined type cannot be checked
                // Bar

                let (checker, state) = create_module_with_foo(StructDefinition {
                    fields: vec![StructDefinitionField {
                        id: "foo",
                        ty: UnresolvedType::FieldElement.mock(),
                    }
                    .mock()],
                });

                assert_eq!(
                    checker.check_type(
                        UnresolvedType::User("Foo".into()).mock(),
                        &PathBuf::from(MODULE_ID).into(),
                        &state.types
                    ),
                    Ok(Type::Struct(StructType::new(
                        "".into(),
                        "Foo".into(),
                        vec![StructMember::new("foo".into(), Type::FieldElement)]
                    )))
                );

                assert_eq!(
                    checker
                        .check_type(
                            UnresolvedType::User("Bar".into()).mock(),
                            &PathBuf::from(MODULE_ID).into(),
                            &state.types
                        )
                        .unwrap_err()
                        .message,
                    "Undefined type Bar"
                );
            }

            #[test]
            fn parameter() {
                // a defined type can be used as parameter

                // an undefined type cannot be used as parameter

                let (checker, state) = create_module_with_foo(StructDefinition {
                    fields: vec![StructDefinitionField {
                        id: "foo",
                        ty: UnresolvedType::FieldElement.mock(),
                    }
                    .mock()],
                });

                assert_eq!(
                    checker.check_parameter(
                        absy::Parameter {
                            id:
                                absy::Variable::new("a", UnresolvedType::User("Foo".into()).mock(),)
                                    .mock(),
                            private: true,
                        }
                        .mock(),
                        &PathBuf::from(MODULE_ID).into(),
                        &state.types,
                    ),
                    Ok(Parameter {
                        id: Variable::with_id_and_type(
                            "a",
                            Type::Struct(StructType::new(
                                "".into(),
                                "Foo".into(),
                                vec![StructMember::new("foo".into(), Type::FieldElement)]
                            ))
                        ),
                        private: true
                    })
                );

                assert_eq!(
                    checker
                        .check_parameter(
                            absy::Parameter {
                                id: absy::Variable::new(
                                    "a",
                                    UnresolvedType::User("Bar".into()).mock(),
                                )
                                .mock(),
                                private: true,
                            }
                            .mock(),
                            &PathBuf::from(MODULE_ID).into(),
                            &state.types,
                        )
                        .unwrap_err()[0]
                        .message,
                    "Undefined type Bar"
                );
            }

            #[test]
            fn variable_declaration() {
                // a defined type can be used in a variable declaration

                // an undefined type cannot be used in a variable declaration

                let (mut checker, state) = create_module_with_foo(StructDefinition {
                    fields: vec![StructDefinitionField {
                        id: "foo",
                        ty: UnresolvedType::FieldElement.mock(),
                    }
                    .mock()],
                });

                assert_eq!(
                    checker.check_statement::<Bn128Field>(
                        Statement::Declaration(
                            absy::Variable::new("a", UnresolvedType::User("Foo".into()).mock(),)
                                .mock()
                        )
                        .mock(),
                        &PathBuf::from(MODULE_ID).into(),
                        &state.types,
                    ),
                    Ok(TypedStatement::Declaration(Variable::with_id_and_type(
                        "a",
                        Type::Struct(StructType::new(
                            "".into(),
                            "Foo".into(),
                            vec![StructMember::new("foo".into(), Type::FieldElement)]
                        ))
                    )))
                );

                assert_eq!(
                    checker
                        .check_parameter(
                            absy::Parameter {
                                id: absy::Variable::new(
                                    "a",
                                    UnresolvedType::User("Bar".into()).mock(),
                                )
                                .mock(),
                                private: true,
                            }
                            .mock(),
                            &PathBuf::from(MODULE_ID).into(),
                            &state.types,
                        )
                        .unwrap_err()[0]
                        .message,
                    "Undefined type Bar"
                );
            }
        }

        /// tests about accessing members
        mod member {
            use super::*;

            #[test]
            fn valid() {
                // accessing a member on a struct should succeed and return the right type

                // struct Foo = { foo: field }
                // Foo { foo: 42 }.foo

                let (mut checker, state) = create_module_with_foo(StructDefinition {
                    fields: vec![StructDefinitionField {
                        id: "foo",
                        ty: UnresolvedType::FieldElement.mock(),
                    }
                    .mock()],
                });

                assert_eq!(
                    checker.check_expression::<Bn128Field>(
                        Expression::Member(
                            box Expression::InlineStruct(
                                "Foo".into(),
                                vec![(
                                    "foo",
                                    Expression::FieldConstant(BigUint::from(42u32)).mock()
                                )]
                            )
                            .mock(),
                            "foo".into()
                        )
                        .mock(),
                        &PathBuf::from(MODULE_ID).into(),
                        &state.types
                    ),
                    Ok(FieldElementExpression::Member(
                        box StructExpressionInner::Value(vec![FieldElementExpression::Number(
                            Bn128Field::from(42u32)
                        )
                        .into()])
                        .annotate(StructType::new(
                            "".into(),
                            "Foo".into(),
                            vec![StructMember::new("foo".into(), Type::FieldElement)]
                        )),
                        "foo".into()
                    )
                    .into())
                );
            }

            #[test]
            fn invalid() {
                // accessing an undefined member on a struct should fail

                // struct Foo = { foo: field }
                // Foo { foo: 42 }.bar

                let (mut checker, state) = create_module_with_foo(StructDefinition {
                    fields: vec![StructDefinitionField {
                        id: "foo",
                        ty: UnresolvedType::FieldElement.mock(),
                    }
                    .mock()],
                });

                assert_eq!(
                    checker
                        .check_expression::<Bn128Field>(
                            Expression::Member(
                                box Expression::InlineStruct(
                                    "Foo".into(),
                                    vec![(
                                        "foo",
                                        Expression::FieldConstant(BigUint::from(42u32)).mock()
                                    )]
                                )
                                .mock(),
                                "bar".into()
                            )
                            .mock(),
                            &PathBuf::from(MODULE_ID).into(),
                            &state.types
                        )
                        .unwrap_err()
                        .message,
                    "Foo {foo: field} doesn\'t have member bar"
                );
            }
        }

        /// tests about defining struct instance inline
        mod value {
            use super::*;

            #[test]
            fn wrong_name() {
                // a A value cannot be defined with B as id, even if A and B have the same members

                let (mut checker, state) = create_module_with_foo(StructDefinition {
                    fields: vec![StructDefinitionField {
                        id: "foo",
                        ty: UnresolvedType::FieldElement.mock(),
                    }
                    .mock()],
                });

                assert_eq!(
                    checker
                        .check_expression::<Bn128Field>(
                            Expression::InlineStruct(
                                "Bar".into(),
                                vec![(
                                    "foo",
                                    Expression::FieldConstant(BigUint::from(42u32)).mock()
                                )]
                            )
                            .mock(),
                            &PathBuf::from(MODULE_ID).into(),
                            &state.types
                        )
                        .unwrap_err()
                        .message,
                    "Undefined type Bar"
                );
            }

            #[test]
            fn valid() {
                // a A value can be defined with members ordered as in the declaration of A

                // struct Foo = { foo: field, bar: bool }
                // Foo foo = Foo { foo: 42, bar: true }

                let (mut checker, state) = create_module_with_foo(StructDefinition {
                    fields: vec![
                        StructDefinitionField {
                            id: "foo",
                            ty: UnresolvedType::FieldElement.mock(),
                        }
                        .mock(),
                        StructDefinitionField {
                            id: "bar",
                            ty: UnresolvedType::Boolean.mock(),
                        }
                        .mock(),
                    ],
                });

                assert_eq!(
                    checker.check_expression::<Bn128Field>(
                        Expression::InlineStruct(
                            "Foo".into(),
                            vec![
                                (
                                    "foo",
                                    Expression::FieldConstant(BigUint::from(42u32)).mock()
                                ),
                                ("bar", Expression::BooleanConstant(true).mock())
                            ]
                        )
                        .mock(),
                        &PathBuf::from(MODULE_ID).into(),
                        &state.types
                    ),
                    Ok(StructExpressionInner::Value(vec![
                        FieldElementExpression::Number(Bn128Field::from(42u32)).into(),
                        BooleanExpression::Value(true).into()
                    ])
                    .annotate(StructType::new(
                        "".into(),
                        "Foo".into(),
                        vec![
                            StructMember::new("foo".into(), Type::FieldElement),
                            StructMember::new("bar".into(), Type::Boolean)
                        ]
                    ))
                    .into())
                );
            }

            #[test]
            fn shuffled() {
                // a A value can be defined with shuffled members compared to the declaration of A

                // struct Foo = { foo: field, bar: bool }
                // Foo foo = Foo { bar: true, foo: 42 }

                let (mut checker, state) = create_module_with_foo(StructDefinition {
                    fields: vec![
                        StructDefinitionField {
                            id: "foo",
                            ty: UnresolvedType::FieldElement.mock(),
                        }
                        .mock(),
                        StructDefinitionField {
                            id: "bar",
                            ty: UnresolvedType::Boolean.mock(),
                        }
                        .mock(),
                    ],
                });

                assert_eq!(
                    checker.check_expression::<Bn128Field>(
                        Expression::InlineStruct(
                            "Foo".into(),
                            vec![
                                ("bar", Expression::BooleanConstant(true).mock()),
                                (
                                    "foo",
                                    Expression::FieldConstant(BigUint::from(42u32)).mock()
                                )
                            ]
                        )
                        .mock(),
                        &PathBuf::from(MODULE_ID).into(),
                        &state.types
                    ),
                    Ok(StructExpressionInner::Value(vec![
                        FieldElementExpression::Number(Bn128Field::from(42u32)).into(),
                        BooleanExpression::Value(true).into()
                    ])
                    .annotate(StructType::new(
                        "".into(),
                        "Foo".into(),
                        vec![
                            StructMember::new("foo".into(), Type::FieldElement),
                            StructMember::new("bar".into(), Type::Boolean)
                        ]
                    ))
                    .into())
                );
            }

            #[test]
            fn subset() {
                // a A value cannot be defined with A as id if members are a subset of the declaration

                // struct Foo = { foo: field, bar: bool }
                // Foo foo = Foo { foo: 42 }

                let (mut checker, state) = create_module_with_foo(StructDefinition {
                    fields: vec![
                        StructDefinitionField {
                            id: "foo",
                            ty: UnresolvedType::FieldElement.mock(),
                        }
                        .mock(),
                        StructDefinitionField {
                            id: "bar",
                            ty: UnresolvedType::Boolean.mock(),
                        }
                        .mock(),
                    ],
                });

                assert_eq!(
                    checker
                        .check_expression::<Bn128Field>(
                            Expression::InlineStruct(
                                "Foo".into(),
                                vec![(
                                    "foo",
                                    Expression::FieldConstant(BigUint::from(42u32)).mock()
                                )]
                            )
                            .mock(),
                            &PathBuf::from(MODULE_ID).into(),
                            &state.types
                        )
                        .unwrap_err()
                        .message,
                    "Inline struct Foo {foo: 42} does not match Foo {foo: field, bar: bool}"
                );
            }

            #[test]
            fn invalid() {
                // a A value cannot be defined with A as id if members are different ids than the declaration
                // a A value cannot be defined with A as id if members are different types than the declaration

                // struct Foo = { foo: field, bar: bool }
                // Foo { foo: 42, baz: bool } // error
                // Foo { foo: 42, baz: 42 } // error

                let (mut checker, state) = create_module_with_foo(StructDefinition {
                    fields: vec![
                        StructDefinitionField {
                            id: "foo",
                            ty: UnresolvedType::FieldElement.mock(),
                        }
                        .mock(),
                        StructDefinitionField {
                            id: "bar",
                            ty: UnresolvedType::Boolean.mock(),
                        }
                        .mock(),
                    ],
                });

                assert_eq!(
                    checker
                        .check_expression::<Bn128Field>(
                            Expression::InlineStruct(
                                "Foo".into(),
                                vec![(
                                    "baz",
                                    Expression::BooleanConstant(true).mock()
                                ),(
                                    "foo",
                                    Expression::FieldConstant(BigUint::from(42u32)).mock()
                                )]
                            )
                            .mock(),
                            &PathBuf::from(MODULE_ID).into(),
                            &state.types
                        ).unwrap_err()
                        .message,
                    "Member bar of struct Foo {foo: field, bar: bool} not found in value Foo {baz: true, foo: 42}"
                );

                assert_eq!(
                    checker
                        .check_expression::<Bn128Field>(
                            Expression::InlineStruct(
                                "Foo".into(),
                                vec![
                                    (
                                        "bar",
                                        Expression::FieldConstant(BigUint::from(42u32)).mock()
                                    ),
                                    (
                                        "foo",
                                        Expression::FieldConstant(BigUint::from(42u32)).mock()
                                    )
                                ]
                            )
                            .mock(),
                            &PathBuf::from(MODULE_ID).into(),
                            &state.types
                        )
                        .unwrap_err()
                        .message,
                    "Member bar of struct Foo has type bool, found 42 of type field"
                );
            }
        }
    }

    mod assignee {
        use super::*;
        use num_bigint::BigUint;

        #[test]
        fn identifier() {
            // a = 42
            let a = Assignee::Identifier("a").mock();

            let types = HashMap::new();
            let module_id = "".into();
            let mut checker: Checker = Checker::new();
            checker
                .check_statement::<Bn128Field>(
                    Statement::Declaration(
                        absy::Variable::new("a", UnresolvedType::FieldElement.mock()).mock(),
                    )
                    .mock(),
                    &module_id,
                    &types,
                )
                .unwrap();

            assert_eq!(
                checker.check_assignee::<Bn128Field>(a, &module_id, &types),
                Ok(TypedAssignee::Identifier(
                    typed_absy::Variable::field_element("a")
                ))
            );
        }

        #[test]
        fn array_element() {
            // field[33] a
            // a[2] = 42
            let a = Assignee::Select(
                box Assignee::Identifier("a").mock(),
                box RangeOrExpression::Expression(
                    Expression::FieldConstant(BigUint::from(2u32)).mock(),
                ),
            )
            .mock();

            let types = HashMap::new();
            let module_id = "".into();

            let mut checker: Checker = Checker::new();
            checker
                .check_statement::<Bn128Field>(
                    Statement::Declaration(
                        absy::Variable::new(
                            "a",
                            UnresolvedType::array(UnresolvedType::FieldElement.mock(), 33).mock(),
                        )
                        .mock(),
                    )
                    .mock(),
                    &module_id,
                    &types,
                )
                .unwrap();

            assert_eq!(
                checker.check_assignee::<Bn128Field>(a, &module_id, &types),
                Ok(TypedAssignee::Select(
                    box TypedAssignee::Identifier(typed_absy::Variable::field_array("a", 33)),
                    box FieldElementExpression::Number(Bn128Field::from(2u32)).into()
                ))
            );
        }

        #[test]
        fn array_of_array_element() {
            // field[33][42] a
            // a[1][2]
            let a = Assignee::Select(
                box Assignee::Select(
                    box Assignee::Identifier("a").mock(),
                    box RangeOrExpression::Expression(
                        Expression::FieldConstant(BigUint::from(1u32)).mock(),
                    ),
                )
                .mock(),
                box RangeOrExpression::Expression(
                    Expression::FieldConstant(BigUint::from(2u32)).mock(),
                ),
            )
            .mock();

            let types = HashMap::new();
            let module_id = "".into();
            let mut checker: Checker = Checker::new();
            checker
                .check_statement::<Bn128Field>(
                    Statement::Declaration(
                        absy::Variable::new(
                            "a",
                            UnresolvedType::array(
                                UnresolvedType::array(UnresolvedType::FieldElement.mock(), 33)
                                    .mock(),
                                42,
                            )
                            .mock(),
                        )
                        .mock(),
                    )
                    .mock(),
                    &module_id,
                    &types,
                )
                .unwrap();

            assert_eq!(
                checker.check_assignee::<Bn128Field>(a, &module_id, &types),
                Ok(TypedAssignee::Select(
                    box TypedAssignee::Select(
                        box TypedAssignee::Identifier(typed_absy::Variable::array(
                            "a",
                            Type::array(Type::FieldElement, 33),
                            42
                        )),
                        box FieldElementExpression::Number(Bn128Field::from(1u32)).into()
                    ),
                    box FieldElementExpression::Number(Bn128Field::from(2u32)).into()
                ))
            );
        }
    }
}
