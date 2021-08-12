// Generic walk through a typed AST. Not mutating in place

use crate::typed_absy::*;
use zokrates_field::Field;

pub trait Folder<'ast, T: Field>: Sized {
    fn fold_program(&mut self, p: TypedProgram<'ast, T>) -> TypedProgram<'ast, T> {
        fold_program(self, p)
    }

    fn fold_module(&mut self, p: TypedModule<'ast, T>) -> TypedModule<'ast, T> {
        fold_module(self, p)
    }

    fn fold_function_symbol(
        &mut self,
        s: TypedFunctionSymbol<'ast, T>,
    ) -> TypedFunctionSymbol<'ast, T> {
        fold_function_symbol(self, s)
    }

    fn fold_function(&mut self, f: TypedFunction<'ast, T>) -> TypedFunction<'ast, T> {
        fold_function(self, f)
    }

    fn fold_parameter(&mut self, p: Parameter<'ast>) -> Parameter<'ast> {
        Parameter {
            id: self.fold_variable(p.id),
            ..p
        }
    }

    fn fold_name(&mut self, n: Identifier<'ast>) -> Identifier<'ast> {
        n
    }

    fn fold_variable(&mut self, v: Variable<'ast>) -> Variable<'ast> {
        Variable {
            id: self.fold_name(v.id),
            ..v
        }
    }

    fn fold_assignee(&mut self, a: TypedAssignee<'ast, T>) -> TypedAssignee<'ast, T> {
        fold_assignee(self, a)
    }

    fn fold_statement(&mut self, s: TypedStatement<'ast, T>) -> Vec<TypedStatement<'ast, T>> {
        fold_statement(self, s)
    }

    fn fold_expression(&mut self, e: TypedExpression<'ast, T>) -> TypedExpression<'ast, T> {
        match e {
            TypedExpression::FieldElement(e) => self.fold_field_expression(e).into(),
            TypedExpression::Boolean(e) => self.fold_boolean_expression(e).into(),
            TypedExpression::Uint(e) => self.fold_uint_expression(e).into(),
            TypedExpression::Array(e) => self.fold_array_expression(e).into(),
            TypedExpression::Struct(e) => self.fold_struct_expression(e).into(),
        }
    }

    fn fold_array_expression(&mut self, e: ArrayExpression<'ast, T>) -> ArrayExpression<'ast, T> {
        fold_array_expression(self, e)
    }

    fn fold_struct_expression(
        &mut self,
        e: StructExpression<'ast, T>,
    ) -> StructExpression<'ast, T> {
        fold_struct_expression(self, e)
    }

    fn fold_expression_list(
        &mut self,
        es: TypedExpressionList<'ast, T>,
    ) -> TypedExpressionList<'ast, T> {
        match es {
            TypedExpressionList::FunctionCall(id, arguments, types) => {
                TypedExpressionList::FunctionCall(
                    id,
                    arguments
                        .into_iter()
                        .map(|a| self.fold_expression(a))
                        .collect(),
                    types,
                )
            }
        }
    }

    fn fold_field_expression(
        &mut self,
        e: FieldElementExpression<'ast, T>,
    ) -> FieldElementExpression<'ast, T> {
        fold_field_expression(self, e)
    }
    fn fold_boolean_expression(
        &mut self,
        e: BooleanExpression<'ast, T>,
    ) -> BooleanExpression<'ast, T> {
        fold_boolean_expression(self, e)
    }
    fn fold_uint_expression(&mut self, e: UExpression<'ast, T>) -> UExpression<'ast, T> {
        fold_uint_expression(self, e)
    }

    fn fold_uint_expression_inner(
        &mut self,
        bitwidth: UBitwidth,
        e: UExpressionInner<'ast, T>,
    ) -> UExpressionInner<'ast, T> {
        fold_uint_expression_inner(self, bitwidth, e)
    }

    fn fold_array_expression_inner(
        &mut self,
        ty: &Type,
        size: usize,
        e: ArrayExpressionInner<'ast, T>,
    ) -> ArrayExpressionInner<'ast, T> {
        fold_array_expression_inner(self, ty, size, e)
    }
    fn fold_struct_expression_inner(
        &mut self,
        ty: &StructType,
        e: StructExpressionInner<'ast, T>,
    ) -> StructExpressionInner<'ast, T> {
        fold_struct_expression_inner(self, ty, e)
    }
}

pub fn fold_module<'ast, T: Field, F: Folder<'ast, T>>(
    f: &mut F,
    p: TypedModule<'ast, T>,
) -> TypedModule<'ast, T> {
    TypedModule {
        functions: p
            .functions
            .into_iter()
            .map(|(key, fun)| (key, f.fold_function_symbol(fun)))
            .collect(),
        ..p
    }
}

pub fn fold_statement<'ast, T: Field, F: Folder<'ast, T>>(
    f: &mut F,
    s: TypedStatement<'ast, T>,
) -> Vec<TypedStatement<'ast, T>> {
    let res = match s {
        TypedStatement::Return(expressions) => TypedStatement::Return(
            expressions
                .into_iter()
                .map(|e| f.fold_expression(e))
                .collect(),
        ),
        TypedStatement::Definition(a, e) => {
            TypedStatement::Definition(f.fold_assignee(a), f.fold_expression(e))
        }
        TypedStatement::Declaration(v) => TypedStatement::Declaration(f.fold_variable(v)),
        TypedStatement::Assertion(e) => TypedStatement::Assertion(f.fold_boolean_expression(e)),
        TypedStatement::For(v, from, to, statements) => TypedStatement::For(
            f.fold_variable(v),
            from,
            to,
            statements
                .into_iter()
                .flat_map(|s| f.fold_statement(s))
                .collect(),
        ),
        TypedStatement::MultipleDefinition(assignees, elist) => TypedStatement::MultipleDefinition(
            assignees.into_iter().map(|a| f.fold_assignee(a)).collect(),
            f.fold_expression_list(elist),
        ),
    };
    vec![res]
}

pub fn fold_array_expression_inner<'ast, T: Field, F: Folder<'ast, T>>(
    f: &mut F,
    _: &Type,
    _: usize,
    e: ArrayExpressionInner<'ast, T>,
) -> ArrayExpressionInner<'ast, T> {
    match e {
        ArrayExpressionInner::Identifier(id) => ArrayExpressionInner::Identifier(f.fold_name(id)),
        ArrayExpressionInner::Value(exprs) => {
            ArrayExpressionInner::Value(exprs.into_iter().map(|e| f.fold_expression(e)).collect())
        }
        ArrayExpressionInner::FunctionCall(id, exps) => {
            let exps = exps.into_iter().map(|e| f.fold_expression(e)).collect();
            ArrayExpressionInner::FunctionCall(id, exps)
        }
        ArrayExpressionInner::IfElse(box condition, box consequence, box alternative) => {
            ArrayExpressionInner::IfElse(
                box f.fold_boolean_expression(condition),
                box f.fold_array_expression(consequence),
                box f.fold_array_expression(alternative),
            )
        }
        ArrayExpressionInner::Member(box s, id) => {
            let s = f.fold_struct_expression(s);
            ArrayExpressionInner::Member(box s, id)
        }
        ArrayExpressionInner::Select(box array, box index) => {
            let array = f.fold_array_expression(array);
            let index = f.fold_field_expression(index);
            ArrayExpressionInner::Select(box array, box index)
        }
    }
}

pub fn fold_struct_expression_inner<'ast, T: Field, F: Folder<'ast, T>>(
    f: &mut F,
    _: &StructType,
    e: StructExpressionInner<'ast, T>,
) -> StructExpressionInner<'ast, T> {
    match e {
        StructExpressionInner::Identifier(id) => StructExpressionInner::Identifier(f.fold_name(id)),
        StructExpressionInner::Value(exprs) => {
            StructExpressionInner::Value(exprs.into_iter().map(|e| f.fold_expression(e)).collect())
        }
        StructExpressionInner::FunctionCall(id, exps) => {
            let exps = exps.into_iter().map(|e| f.fold_expression(e)).collect();
            StructExpressionInner::FunctionCall(id, exps)
        }
        StructExpressionInner::IfElse(box condition, box consequence, box alternative) => {
            StructExpressionInner::IfElse(
                box f.fold_boolean_expression(condition),
                box f.fold_struct_expression(consequence),
                box f.fold_struct_expression(alternative),
            )
        }
        StructExpressionInner::Member(box s, id) => {
            let s = f.fold_struct_expression(s);
            StructExpressionInner::Member(box s, id)
        }
        StructExpressionInner::Select(box array, box index) => {
            let array = f.fold_array_expression(array);
            let index = f.fold_field_expression(index);
            StructExpressionInner::Select(box array, box index)
        }
    }
}

pub fn fold_field_expression<'ast, T: Field, F: Folder<'ast, T>>(
    f: &mut F,
    e: FieldElementExpression<'ast, T>,
) -> FieldElementExpression<'ast, T> {
    match e {
        FieldElementExpression::Number(n) => FieldElementExpression::Number(n),
        FieldElementExpression::Identifier(id) => {
            FieldElementExpression::Identifier(f.fold_name(id))
        }
        FieldElementExpression::Add(box e1, box e2) => {
            let e1 = f.fold_field_expression(e1);
            let e2 = f.fold_field_expression(e2);
            FieldElementExpression::Add(box e1, box e2)
        }
        FieldElementExpression::Sub(box e1, box e2) => {
            let e1 = f.fold_field_expression(e1);
            let e2 = f.fold_field_expression(e2);
            FieldElementExpression::Sub(box e1, box e2)
        }
        FieldElementExpression::Mult(box e1, box e2) => {
            let e1 = f.fold_field_expression(e1);
            let e2 = f.fold_field_expression(e2);
            FieldElementExpression::Mult(box e1, box e2)
        }
        FieldElementExpression::Div(box e1, box e2) => {
            let e1 = f.fold_field_expression(e1);
            let e2 = f.fold_field_expression(e2);
            FieldElementExpression::Div(box e1, box e2)
        }
        FieldElementExpression::Pow(box e1, box e2) => {
            let e1 = f.fold_field_expression(e1);
            let e2 = f.fold_field_expression(e2);
            FieldElementExpression::Pow(box e1, box e2)
        }
        FieldElementExpression::IfElse(box cond, box cons, box alt) => {
            let cond = f.fold_boolean_expression(cond);
            let cons = f.fold_field_expression(cons);
            let alt = f.fold_field_expression(alt);
            FieldElementExpression::IfElse(box cond, box cons, box alt)
        }
        FieldElementExpression::FunctionCall(key, exps) => {
            let exps = exps.into_iter().map(|e| f.fold_expression(e)).collect();
            FieldElementExpression::FunctionCall(key, exps)
        }
        FieldElementExpression::Member(box s, id) => {
            let s = f.fold_struct_expression(s);
            FieldElementExpression::Member(box s, id)
        }
        FieldElementExpression::Select(box array, box index) => {
            let array = f.fold_array_expression(array);
            let index = f.fold_field_expression(index);
            FieldElementExpression::Select(box array, box index)
        }
    }
}

pub fn fold_boolean_expression<'ast, T: Field, F: Folder<'ast, T>>(
    f: &mut F,
    e: BooleanExpression<'ast, T>,
) -> BooleanExpression<'ast, T> {
    match e {
        BooleanExpression::Value(v) => BooleanExpression::Value(v),
        BooleanExpression::Identifier(id) => BooleanExpression::Identifier(f.fold_name(id)),
        BooleanExpression::FieldEq(box e1, box e2) => {
            let e1 = f.fold_field_expression(e1);
            let e2 = f.fold_field_expression(e2);
            BooleanExpression::FieldEq(box e1, box e2)
        }
        BooleanExpression::BoolEq(box e1, box e2) => {
            let e1 = f.fold_boolean_expression(e1);
            let e2 = f.fold_boolean_expression(e2);
            BooleanExpression::BoolEq(box e1, box e2)
        }
        BooleanExpression::ArrayEq(box e1, box e2) => {
            let e1 = f.fold_array_expression(e1);
            let e2 = f.fold_array_expression(e2);
            BooleanExpression::ArrayEq(box e1, box e2)
        }
        BooleanExpression::StructEq(box e1, box e2) => {
            let e1 = f.fold_struct_expression(e1);
            let e2 = f.fold_struct_expression(e2);
            BooleanExpression::StructEq(box e1, box e2)
        }
        BooleanExpression::UintEq(box e1, box e2) => {
            let e1 = f.fold_uint_expression(e1);
            let e2 = f.fold_uint_expression(e2);
            BooleanExpression::UintEq(box e1, box e2)
        }
        BooleanExpression::Lt(box e1, box e2) => {
            let e1 = f.fold_field_expression(e1);
            let e2 = f.fold_field_expression(e2);
            BooleanExpression::Lt(box e1, box e2)
        }
        BooleanExpression::Le(box e1, box e2) => {
            let e1 = f.fold_field_expression(e1);
            let e2 = f.fold_field_expression(e2);
            BooleanExpression::Le(box e1, box e2)
        }
        BooleanExpression::Gt(box e1, box e2) => {
            let e1 = f.fold_field_expression(e1);
            let e2 = f.fold_field_expression(e2);
            BooleanExpression::Gt(box e1, box e2)
        }
        BooleanExpression::Ge(box e1, box e2) => {
            let e1 = f.fold_field_expression(e1);
            let e2 = f.fold_field_expression(e2);
            BooleanExpression::Ge(box e1, box e2)
        }
        BooleanExpression::Or(box e1, box e2) => {
            let e1 = f.fold_boolean_expression(e1);
            let e2 = f.fold_boolean_expression(e2);
            BooleanExpression::Or(box e1, box e2)
        }
        BooleanExpression::And(box e1, box e2) => {
            let e1 = f.fold_boolean_expression(e1);
            let e2 = f.fold_boolean_expression(e2);
            BooleanExpression::And(box e1, box e2)
        }
        BooleanExpression::Not(box e) => {
            let e = f.fold_boolean_expression(e);
            BooleanExpression::Not(box e)
        }
        BooleanExpression::FunctionCall(key, exps) => {
            let exps = exps.into_iter().map(|e| f.fold_expression(e)).collect();
            BooleanExpression::FunctionCall(key, exps)
        }
        BooleanExpression::IfElse(box cond, box cons, box alt) => {
            let cond = f.fold_boolean_expression(cond);
            let cons = f.fold_boolean_expression(cons);
            let alt = f.fold_boolean_expression(alt);
            BooleanExpression::IfElse(box cond, box cons, box alt)
        }
        BooleanExpression::Member(box s, id) => {
            let s = f.fold_struct_expression(s);
            BooleanExpression::Member(box s, id)
        }
        BooleanExpression::Select(box array, box index) => {
            let array = f.fold_array_expression(array);
            let index = f.fold_field_expression(index);
            BooleanExpression::Select(box array, box index)
        }
    }
}

pub fn fold_uint_expression<'ast, T: Field, F: Folder<'ast, T>>(
    f: &mut F,
    e: UExpression<'ast, T>,
) -> UExpression<'ast, T> {
    UExpression {
        inner: f.fold_uint_expression_inner(e.bitwidth, e.inner),
        ..e
    }
}

pub fn fold_uint_expression_inner<'ast, T: Field, F: Folder<'ast, T>>(
    f: &mut F,
    _: UBitwidth,
    e: UExpressionInner<'ast, T>,
) -> UExpressionInner<'ast, T> {
    match e {
        UExpressionInner::Value(v) => UExpressionInner::Value(v),
        UExpressionInner::Identifier(id) => UExpressionInner::Identifier(f.fold_name(id)),
        UExpressionInner::Add(box left, box right) => {
            let left = f.fold_uint_expression(left);
            let right = f.fold_uint_expression(right);

            UExpressionInner::Add(box left, box right)
        }
        UExpressionInner::Sub(box left, box right) => {
            let left = f.fold_uint_expression(left);
            let right = f.fold_uint_expression(right);

            UExpressionInner::Sub(box left, box right)
        }
        UExpressionInner::Mult(box left, box right) => {
            let left = f.fold_uint_expression(left);
            let right = f.fold_uint_expression(right);

            UExpressionInner::Mult(box left, box right)
        }
        UExpressionInner::Div(box left, box right) => {
            let left = f.fold_uint_expression(left);
            let right = f.fold_uint_expression(right);

            UExpressionInner::Div(box left, box right)
        }
        UExpressionInner::Rem(box left, box right) => {
            let left = f.fold_uint_expression(left);
            let right = f.fold_uint_expression(right);

            UExpressionInner::Rem(box left, box right)
        }
        UExpressionInner::Xor(box left, box right) => {
            let left = f.fold_uint_expression(left);
            let right = f.fold_uint_expression(right);

            UExpressionInner::Xor(box left, box right)
        }
        UExpressionInner::And(box left, box right) => {
            let left = f.fold_uint_expression(left);
            let right = f.fold_uint_expression(right);

            UExpressionInner::And(box left, box right)
        }
        UExpressionInner::Or(box left, box right) => {
            let left = f.fold_uint_expression(left);
            let right = f.fold_uint_expression(right);

            UExpressionInner::Or(box left, box right)
        }
        UExpressionInner::LeftShift(box e, box by) => {
            let e = f.fold_uint_expression(e);
            let by = f.fold_field_expression(by);

            UExpressionInner::LeftShift(box e, box by)
        }
        UExpressionInner::RightShift(box e, box by) => {
            let e = f.fold_uint_expression(e);
            let by = f.fold_field_expression(by);

            UExpressionInner::RightShift(box e, box by)
        }
        UExpressionInner::Not(box e) => {
            let e = f.fold_uint_expression(e);

            UExpressionInner::Not(box e)
        }
        UExpressionInner::FunctionCall(key, exps) => {
            let exps = exps.into_iter().map(|e| f.fold_expression(e)).collect();
            UExpressionInner::FunctionCall(key, exps)
        }
        UExpressionInner::Select(box array, box index) => {
            let array = f.fold_array_expression(array);
            let index = f.fold_field_expression(index);
            UExpressionInner::Select(box array, box index)
        }
        UExpressionInner::IfElse(box cond, box cons, box alt) => {
            let cond = f.fold_boolean_expression(cond);
            let cons = f.fold_uint_expression(cons);
            let alt = f.fold_uint_expression(alt);
            UExpressionInner::IfElse(box cond, box cons, box alt)
        }
        UExpressionInner::Member(box s, id) => {
            let s = f.fold_struct_expression(s);
            UExpressionInner::Member(box s, id)
        }
    }
}

pub fn fold_function<'ast, T: Field, F: Folder<'ast, T>>(
    f: &mut F,
    fun: TypedFunction<'ast, T>,
) -> TypedFunction<'ast, T> {
    TypedFunction {
        arguments: fun
            .arguments
            .into_iter()
            .map(|a| f.fold_parameter(a))
            .collect(),
        statements: fun
            .statements
            .into_iter()
            .flat_map(|s| f.fold_statement(s))
            .collect(),
        ..fun
    }
}

pub fn fold_array_expression<'ast, T: Field, F: Folder<'ast, T>>(
    f: &mut F,
    e: ArrayExpression<'ast, T>,
) -> ArrayExpression<'ast, T> {
    ArrayExpression {
        inner: f.fold_array_expression_inner(&e.ty, e.size, e.inner),
        ..e
    }
}

pub fn fold_struct_expression<'ast, T: Field, F: Folder<'ast, T>>(
    f: &mut F,
    e: StructExpression<'ast, T>,
) -> StructExpression<'ast, T> {
    StructExpression {
        inner: f.fold_struct_expression_inner(&e.ty, e.inner),
        ..e
    }
}

pub fn fold_function_symbol<'ast, T: Field, F: Folder<'ast, T>>(
    f: &mut F,
    s: TypedFunctionSymbol<'ast, T>,
) -> TypedFunctionSymbol<'ast, T> {
    match s {
        TypedFunctionSymbol::Here(fun) => TypedFunctionSymbol::Here(f.fold_function(fun)),
        there => there, // by default, do not fold modules recursively
    }
}

pub fn fold_assignee<'ast, T: Field, F: Folder<'ast, T>>(
    f: &mut F,
    a: TypedAssignee<'ast, T>,
) -> TypedAssignee<'ast, T> {
    match a {
        TypedAssignee::Identifier(v) => TypedAssignee::Identifier(f.fold_variable(v)),
        TypedAssignee::Select(box a, box index) => {
            TypedAssignee::Select(box f.fold_assignee(a), box f.fold_field_expression(index))
        }
        TypedAssignee::Member(box s, m) => TypedAssignee::Member(box f.fold_assignee(s), m),
    }
}

pub fn fold_program<'ast, T: Field, F: Folder<'ast, T>>(
    f: &mut F,
    p: TypedProgram<'ast, T>,
) -> TypedProgram<'ast, T> {
    TypedProgram {
        modules: p
            .modules
            .into_iter()
            .map(|(module_id, module)| (module_id, f.fold_module(module)))
            .collect(),
        main: p.main,
    }
}
