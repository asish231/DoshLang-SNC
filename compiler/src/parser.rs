use crate::ast::*;
use crate::lexer::Lexer;
use crate::span::Span;
use crate::token::{Token, TokenKind};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    file: u32,
}

impl Parser {
    pub fn parse_file(file: u32, src: &str) -> Result<Program, String> {
        let mut lexer = Lexer::new(file, src);
        let tokens = lexer.tokenize()?;
        let mut p = Parser {
            tokens,
            pos: 0,
            file,
        };
        p.parse_program()
    }

    fn parse_program(&mut self) -> Result<Program, String> {
        let mut items = Vec::new();
        while !self.at(TokenKind::Eof) {
            items.push(self.parse_item()?);
        }
        Ok(Program {
            file: self.file,
            items,
        })
    }

    fn parse_item(&mut self) -> Result<Item, String> {
        match self.peek_kind() {
            TokenKind::Use => Ok(Item::Use(self.parse_use()?)),
            TokenKind::Async | TokenKind::Fn => Ok(Item::Fn(self.parse_fn()?)),
            TokenKind::Blueprint => Ok(Item::Blueprint(self.parse_blueprint()?)),
            TokenKind::Record => Ok(Item::Record(self.parse_record()?)),
            TokenKind::PackedKw => Ok(Item::Record(self.parse_packed_record()?)),
            TokenKind::EnumKw => Ok(Item::Enum(self.parse_enum()?)),
            TokenKind::Extension => Ok(Item::Extension(self.parse_extension()?)),
            TokenKind::Extern => Ok(Item::Extern(self.parse_extern()?)),
            TokenKind::Contract => Ok(Item::Contract(self.parse_contract()?)),
            _ => Ok(Item::Stmt(self.parse_stmt()?)),
        }
    }

    fn parse_use(&mut self) -> Result<UseItem, String> {
        let start = self.eat(TokenKind::Use)?;
        let mut path = vec![self.expect_ident()?];
        while self.at(TokenKind::Dot) {
            self.bump();
            path.push(self.expect_ident()?);
        }
        Ok(UseItem {
            span: start.merge(self.prev_span()),
            path,
        })
    }

    /// `fn name(params) -> T` with no body, for `extern` blocks.
    fn parse_extern_fn(&mut self) -> Result<FnItem, String> {
        let start_span = self.peek().span;
        self.eat(TokenKind::Fn)?;
        let name = self.expect_ident()?;
        self.eat(TokenKind::LParen)?;
        let mut params = Vec::new();
        if !self.at(TokenKind::RParen) {
            loop {
                params.push(self.parse_param()?);
                if !self.eat_if(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.eat(TokenKind::RParen)?;
        let ret = if self.eat_if(TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        Ok(FnItem {
            is_async: false,
            is_static: false,
            is_abstract: true,
            access: Access::Open,
            name,
            params,
            ret,
            body: Vec::new(),
            span: start_span.merge(self.prev_span()),
        })
    }

    fn parse_fn(&mut self) -> Result<FnItem, String> {
        let start_span = self.peek().span;
        let is_abstract = self.eat_if(TokenKind::Abstract);
        let access = if self.eat_if(TokenKind::Open) {
            Access::Open
        } else if self.eat_if(TokenKind::Closed) {
            Access::Closed
        } else if self.eat_if(TokenKind::Guarded) {
            Access::Guarded
        } else {
            Access::Open
        };
        let is_async = self.eat_if(TokenKind::Async);
        self.eat(TokenKind::Fn)?;
        let name = self.expect_ident()?;
        self.eat(TokenKind::LParen)?;
        let mut params = Vec::new();
        if !self.at(TokenKind::RParen) {
            loop {
                params.push(self.parse_param()?);
                if !self.eat_if(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.eat(TokenKind::RParen)?;
        let ret = if self.eat_if(TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        // `abstract fn f() -> T` declares a signature with no body.
        let body = if is_abstract {
            Vec::new()
        } else {
            let parsed = self.parse_block();
            match parsed {
                Ok(b) => b,
                Err(e) => {
                    return Err(format!(
                        "{e} (a method with no body must be declared `abstract fn {name}`)"
                    ))
                }
            }
        };
        Ok(FnItem {
            is_async,
            is_static: false,
            is_abstract,
            access,
            name,
            params,
            ret,
            body,
            span: start_span.merge(self.prev_span()),
        })
    }

    fn parse_param(&mut self) -> Result<Param, String> {
        let ty = self.parse_type()?;
        let name = self.expect_ident()?;
        let default = if self.eat_if(TokenKind::Eq) {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Param { ty, name, default })
    }

    fn parse_blueprint(&mut self) -> Result<BlueprintItem, String> {
        let start = self.eat(TokenKind::Blueprint)?;
        let name = self.expect_ident()?;
        let mut type_params = Vec::new();
        if self.eat_if(TokenKind::Lt) {
            type_params.push(self.parse_type_param()?);
            while self.eat_if(TokenKind::Comma) {
                type_params.push(self.parse_type_param()?);
            }
            self.eat_gt()?;
        }
        let mut parent = None;
        let mut contracts = Vec::new();
        if self.eat_if(TokenKind::From) {
            parent = Some(self.expect_ident()?);
        }
        if self.eat_if(TokenKind::Follows) {
            contracts.push(self.expect_ident()?);
            while self.eat_if(TokenKind::Comma) {
                contracts.push(self.expect_ident()?);
            }
        }
        self.eat(TokenKind::LBrace)?;
        let mut fields = Vec::new();
        let mut methods = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            if self.at(TokenKind::Static) {
                self.bump();
                let mut m = self.parse_fn()?;
                m.is_static = true;
                methods.push(m);
            } else if self.at(TokenKind::Fn)
                || self.at(TokenKind::Async)
                || self.at(TokenKind::Abstract)
                || self.at(TokenKind::Open)
                || self.at(TokenKind::Closed)
                || self.at(TokenKind::Guarded)
            {
                methods.push(self.parse_fn()?);
            } else {
                fields.push(self.parse_field()?);
            }
        }
        self.eat(TokenKind::RBrace)?;
        Ok(BlueprintItem {
            name,
            type_params,
            parent,
            contracts,
            fields,
            methods,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_record(&mut self) -> Result<RecordItem, String> {
        let start = self.eat(TokenKind::Record)?;
        let name = self.expect_ident()?;
        self.eat(TokenKind::LBrace)?;
        let mut fields = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            fields.push(self.parse_field()?);
        }
        self.eat(TokenKind::RBrace)?;
        Ok(RecordItem {
            name,
            fields,
            packed: false,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_packed_record(&mut self) -> Result<RecordItem, String> {
        let start = self.eat(TokenKind::PackedKw)?;
        self.eat(TokenKind::Record)?;
        let name = self.expect_ident()?;
        self.eat(TokenKind::LBrace)?;
        let mut fields = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            fields.push(self.parse_field()?);
        }
        self.eat(TokenKind::RBrace)?;
        Ok(RecordItem {
            name,
            fields,
            packed: true,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_enum(&mut self) -> Result<EnumItem, String> {
        let start = self.eat(TokenKind::EnumKw)?;
        let name = self.expect_ident()?;
        self.eat(TokenKind::LBrace)?;
        let mut variants = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let v = self.expect_ident()?;
            variants.push(v);
            if !self.eat_if(TokenKind::Comma) {
                break;
            }
        }
        self.eat(TokenKind::RBrace)?;
        if variants.is_empty() {
            return Err(self.error("enum needs at least one variant"));
        }
        Ok(EnumItem {
            name,
            variants,
            span: start.merge(self.prev_span()),
        })
    }

    /// `extern "sqlite3" { fn sqlite3_open(ptr path, ptr db) -> int }`
    fn parse_extern(&mut self) -> Result<ExternItem, String> {
        let start = self.eat(TokenKind::Extern)?;
        let lib = match &self.peek().kind {
            TokenKind::String => {
                let t = self.bump();
                t.text.trim_matches('"').to_string()
            }
            _ => String::new(),
        };
        self.eat(TokenKind::LBrace)?;
        let mut funcs = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            funcs.push(self.parse_extern_fn()?);
        }
        self.eat(TokenKind::RBrace)?;
        Ok(ExternItem {
            lib,
            funcs,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_extension(&mut self) -> Result<ExtensionItem, String> {
        let start = self.eat(TokenKind::Extension)?;
        let ty = self.parse_type()?;
        self.eat(TokenKind::LBrace)?;
        let mut methods = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            if self.at(TokenKind::Static) {
                self.bump(); // consume "static"
                let mut m = self.parse_fn()?;
                m.is_static = true;
                methods.push(m);
            } else {
                methods.push(self.parse_fn()?);
            }
        }
        self.eat(TokenKind::RBrace)?;
        Ok(ExtensionItem {
            ty,
            methods,
            span: start.merge(self.prev_span()),
        })
    }

    /// `T`, `T: Bound`, `out T`, `in T`, or `out T: Bound`.
    ///
    /// `out` / `in` are contextual keywords: they only introduce a variance
    /// here, so `str out = ""` still parses as an ordinary declaration.
    fn parse_type_param(&mut self) -> Result<TypeParam, String> {
        let variance = if self.at_variance_marker() {
            if self.peek().text == "out" {
                self.bump();
                Variance::Covariant
            } else {
                self.bump();
                Variance::Contravariant
            }
        } else {
            Variance::Invariant
        };
        let name = self.expect_ident()?;
        let bound = if self.eat(TokenKind::Colon).is_ok() {
            Some(self.expect_ident()?)
        } else {
            None
        };
        Ok(TypeParam {
            name,
            bound,
            variance,
        })
    }

    fn parse_field(&mut self) -> Result<Field, String> {
        let access = if self.eat_if(TokenKind::Open) {
            Access::Open
        } else if self.eat_if(TokenKind::Closed) {
            Access::Closed
        } else if self.eat_if(TokenKind::Guarded) {
            Access::Guarded
        } else {
            Access::Open
        };
        let ty = self.parse_type()?;
        let name = self.expect_ident()?;
        let init = if self.eat_if(TokenKind::Eq) {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Field {
            access,
            ty,
            name,
            init,
        })
    }

    fn parse_contract(&mut self) -> Result<ContractItem, String> {
        let start = self.eat(TokenKind::Contract)?;
        let name = self.expect_ident()?;
        self.eat(TokenKind::LBrace)?;
        let mut methods = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            let mstart = self.eat(TokenKind::Fn)?;
            let mname = self.expect_ident()?;
            self.eat(TokenKind::LParen)?;
            let mut params = Vec::new();
            if !self.at(TokenKind::RParen) {
                loop {
                    params.push(self.parse_param()?);
                    if !self.eat_if(TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.eat(TokenKind::RParen)?;
            let ret = if self.eat_if(TokenKind::Arrow) {
                Some(self.parse_type()?)
            } else {
                None
            };
            methods.push(ContractMethod {
                name: mname,
                params,
                ret,
                span: mstart.merge(self.prev_span()),
            });
        }
        self.eat(TokenKind::RBrace)?;
        Ok(ContractItem {
            name,
            methods,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_type(&mut self) -> Result<TypeAst, String> {
        let mut ty = self.parse_type_base()?;
        if self.eat_if(TokenKind::Question) {
            ty = TypeAst::Optional(Box::new(ty));
        }
        Ok(ty)
    }

    fn parse_type_base(&mut self) -> Result<TypeAst, String> {
        if self.eat_if(TokenKind::LParen) {
            let mut parts = vec![self.parse_type()?];
            self.eat(TokenKind::Comma)?;
            parts.push(self.parse_type()?);
            while self.eat_if(TokenKind::Comma) {
                parts.push(self.parse_type()?);
            }
            self.eat(TokenKind::RParen)?;
            return Ok(TypeAst::Tuple(parts));
        }
        match self.peek_kind() {
            TokenKind::IntKw => {
                self.bump();
                Ok(TypeAst::Int)
            }
            TokenKind::I8Kw => {
                self.bump();
                Ok(TypeAst::I8)
            }
            TokenKind::I16Kw => {
                self.bump();
                Ok(TypeAst::I16)
            }
            TokenKind::I32Kw => {
                self.bump();
                Ok(TypeAst::I32)
            }
            TokenKind::U8Kw => {
                self.bump();
                Ok(TypeAst::U8)
            }
            TokenKind::U16Kw => {
                self.bump();
                Ok(TypeAst::U16)
            }
            TokenKind::U32Kw => {
                self.bump();
                Ok(TypeAst::U32)
            }
            TokenKind::U64Kw => {
                self.bump();
                Ok(TypeAst::U64)
            }
            TokenKind::FloatKw => {
                self.bump();
                Ok(TypeAst::Float)
            }
            TokenKind::StrKw => {
                self.bump();
                Ok(TypeAst::Str)
            }
            TokenKind::BoolKw => {
                self.bump();
                Ok(TypeAst::Bool)
            }
            TokenKind::ByteKw => {
                self.bump();
                Ok(TypeAst::Byte)
            }
            TokenKind::ErrorKw => {
                self.bump();
                Ok(TypeAst::Error)
            }
            TokenKind::DecKw => {
                self.bump();
                self.eat(TokenKind::LParen)?;
                let n = self.expect_int()?;
                self.eat(TokenKind::RParen)?;
                Ok(TypeAst::Dec(n as u32))
            }
            TokenKind::ListKw => {
                self.bump();
                self.eat(TokenKind::Lt)?;
                let inner = self.parse_type()?;
                self.eat_gt()?;
                Ok(TypeAst::List(Box::new(inner)))
            }
            TokenKind::MapKw => {
                self.bump();
                self.eat(TokenKind::Lt)?;
                let k = self.parse_type()?;
                self.eat(TokenKind::Comma)?;
                let v = self.parse_type()?;
                self.eat_gt()?;
                Ok(TypeAst::Map(Box::new(k), Box::new(v)))
            }
            TokenKind::Chan => {
                self.bump();
                self.eat(TokenKind::Lt)?;
                let inner = self.parse_type()?;
                self.eat_gt()?;
                Ok(TypeAst::Chan(Box::new(inner)))
            }
            TokenKind::RefKw => {
                self.bump();
                self.eat(TokenKind::Lt)?;
                let inner = self.parse_type()?;
                self.eat_gt()?;
                Ok(TypeAst::Ref(Box::new(inner)))
            }
            TokenKind::Fn => {
                self.bump();
                self.eat(TokenKind::LParen)?;
                let mut params = Vec::new();
                if !self.at(TokenKind::RParen) {
                    loop {
                        params.push(self.parse_type()?);
                        if !self.eat_if(TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.eat(TokenKind::RParen)?;
                let ret = if self.eat_if(TokenKind::Arrow) {
                    Some(Box::new(self.parse_type()?))
                } else {
                    None
                };
                Ok(TypeAst::Fn { params, ret })
            }
            TokenKind::AnyKw => {
                self.bump();
                Ok(TypeAst::Any)
            }
            TokenKind::Ptr => {
                self.bump();
                Ok(TypeAst::Ptr)
            }
            TokenKind::Mut => {
                self.bump();
                self.eat(TokenKind::Lt)?;
                let inner = self.parse_type()?;
                self.eat_gt()?;
                Ok(TypeAst::Mut(Box::new(inner)))
            }
            TokenKind::Ident => {
                let name = self.expect_ident()?;
                if self.at(TokenKind::Lt) {
                    // `Foo<Bar>` in type position. Guard against `a < b`
                    // comparisons: only treat `<` as a type argument list when
                    // a matching `>` closes it.
                    if self.looks_like_type_args() {
                        self.bump(); // `<`
                        let mut args = Vec::new();
                        if !self.at(TokenKind::Gt) {
                            loop {
                                args.push(self.parse_type()?);
                                if !self.eat_if(TokenKind::Comma) {
                                    break;
                                }
                            }
                        }
                        self.eat_gt()?;
                        return Ok(TypeAst::Generic(name, args));
                    }
                }
                Ok(TypeAst::Named(name))
            }
            _ => Err(self.error("expected type")),
        }
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, String> {
        self.eat(TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            stmts.push(self.parse_stmt()?);
        }
        self.eat(TokenKind::RBrace)?;
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        match self.peek_kind() {
            TokenKind::If => self.parse_if(),
            TokenKind::While => self.parse_while(),
            TokenKind::For => self.parse_for(),
            TokenKind::Match => self.parse_match(),
            TokenKind::Return => {
                let span = self.bump().span;
                let value = if self.starts_expr() {
                    Some(self.parse_expr()?)
                } else {
                    None
                };
                Ok(Stmt::Return { value, span })
            }
            TokenKind::Stop => {
                let span = self.bump().span;
                Ok(Stmt::Stop(span))
            }
            TokenKind::Skip => {
                let span = self.bump().span;
                Ok(Stmt::Skip(span))
            }
            TokenKind::Spawn => self.parse_spawn(),
            TokenKind::Goroutine => self.parse_goroutine(),
            TokenKind::Lock => self.parse_lock_stmt(),
            TokenKind::Asm => self.parse_asm(),
            TokenKind::New => self.parse_new(true),
            TokenKind::Const => self.parse_decl(true),
            TokenKind::Let => self.parse_decl(false),
            TokenKind::Chan => self.parse_typed_decl(),
            TokenKind::Defer => {
                let span = self.bump().span;
                let body = self.parse_block()?;
                Ok(Stmt::Defer {
                    span: span.merge(self.prev_span()),
                    body,
                })
            }
            TokenKind::Fn | TokenKind::Async => {
                if self.at(TokenKind::Fn) && matches!(self.peek_n(1), TokenKind::LParen) {
                    let expr = self.parse_expr()?;
                    self.finish_expr_stmt(expr)
                } else {
                    Ok(Stmt::NestedFn(self.parse_fn()?))
                }
            }
            k if is_type_start(k) => {
                if self.looks_like_new_without_keyword() {
                    self.parse_new(false)
                } else if self.looks_like_typed_decl() {
                    self.parse_typed_decl()
                } else {
                    let expr = self.parse_expr()?;
                    self.finish_expr_stmt(expr)
                }
            }
            _ => {
                let expr = self.parse_expr()?;
                self.finish_expr_stmt(expr)
            }
        }
    }

    fn finish_expr_stmt(&mut self, expr: Expr) -> Result<Stmt, String> {
        if let Some(op) = self.assign_op() {
            self.bump();
            let value = self.parse_expr()?;
            let span = expr.span.merge(value.span);
            return Ok(Stmt::Assign {
                target: expr,
                op,
                value,
                span,
            });
        }
        if self.at(TokenKind::Comma) {
            let mut names = match &expr.kind {
                ExprKind::Ident(n) => vec![n.clone()],
                _ => return Err(self.error("tuple assignment requires identifiers")),
            };
            while self.eat_if(TokenKind::Comma) {
                names.push(self.expect_ident()?);
            }
            self.eat(TokenKind::Eq)?;
            let value = self.parse_expr()?;
            let span = expr.span.merge(value.span);
            let els: Vec<Expr> = names
                .into_iter()
                .map(|n| Expr::ident(n, span))
                .collect();
            return Ok(Stmt::Assign {
                target: Expr {
                    kind: ExprKind::Tuple(els),
                    span,
                },
                op: AssignOp::Eq,
                value,
                span,
            });
        }
        Ok(Stmt::Expr(expr))
    }

    fn looks_like_typed_decl(&self) -> bool {
        match self.peek_kind() {
            TokenKind::IntKw
            | TokenKind::I8Kw
            | TokenKind::I16Kw
            | TokenKind::I32Kw
            | TokenKind::U8Kw
            | TokenKind::U16Kw
            | TokenKind::U32Kw
            | TokenKind::U64Kw
            | TokenKind::FloatKw
            | TokenKind::StrKw
            | TokenKind::BoolKw
            | TokenKind::ByteKw
            | TokenKind::DecKw
            | TokenKind::ListKw
            | TokenKind::MapKw
            | TokenKind::Chan
            | TokenKind::RefKw
            | TokenKind::ErrorKw
            | TokenKind::Ptr
            | TokenKind::Mut
            | TokenKind::LParen => true,
            TokenKind::Fn => true,
            TokenKind::Ident => {
                matches!(self.peek_n(1), TokenKind::Ident | TokenKind::Question)
                    // `Foo<Bar> x = ...` — generic type application in a decl
                    || (self.peek_n(1) == TokenKind::Lt && self.looks_like_type_args_at(1))
            }
            _ => false,
        }
    }

    /// Distinguish `Foo<Bar>` (type application) from `a < b` (comparison) by
    /// scanning forward for a `>` that closes before any token that cannot
    /// appear inside a type argument list.
    fn looks_like_type_args(&self) -> bool {
        self.looks_like_type_args_at(1)
    }

    /// `at` is the index of the `<` token relative to the current position.
    fn looks_like_type_args_at(&self, at: usize) -> bool {
        let mut i = at + 1; // first token after `<`
        let mut depth = 1i32;
        let mut steps = 0usize;
        loop {
            steps += 1;
            if steps > 64 {
                return false;
            }
            match self.peek_n(i) {
                TokenKind::Lt => depth += 1,
                TokenKind::Gt => {
                    depth -= 1;
                    if depth == 0 {
                        return true;
                    }
                }
                TokenKind::GtEq | TokenKind::PlusEq | TokenKind::MinusEq
                | TokenKind::StarEq | TokenKind::SlashEq => return false,
                TokenKind::LBrace | TokenKind::RBrace | TokenKind::Eof => return false,
                _ => {}
            }
            i += 1;
        }
    }

    fn looks_like_new_without_keyword(&self) -> bool {
        // Type name (
        if !matches!(
            self.peek_kind(),
            TokenKind::Ident
                | TokenKind::IntKw
                | TokenKind::I8Kw
                | TokenKind::I16Kw
                | TokenKind::I32Kw
                | TokenKind::U8Kw
                | TokenKind::U16Kw
                | TokenKind::U32Kw
                | TokenKind::U64Kw
                | TokenKind::FloatKw
                | TokenKind::StrKw
                | TokenKind::BoolKw
                | TokenKind::ByteKw
        ) {
            return false;
        }
        matches!(self.peek_n(1), TokenKind::Ident) && matches!(self.peek_n(2), TokenKind::LParen)
    }

    fn parse_new(&mut self, heap: bool) -> Result<Stmt, String> {
        let start = self.peek().span;
        if heap {
            self.eat(TokenKind::New)?;
        }
        let ty = self.expect_ident()?;
        let mut type_args = Vec::new();
        if self.eat_if(TokenKind::Lt) {
            type_args.push(self.parse_type()?);
            while self.eat_if(TokenKind::Comma) {
                type_args.push(self.parse_type()?);
            }
            self.eat_gt()?;
        }
        let name = self.expect_ident()?;
        self.eat(TokenKind::LParen)?;
        let args = self.parse_arg_list()?;
        self.eat(TokenKind::RParen)?;
        Ok(Stmt::New {
            heap,
            ty,
            type_args,
            name,
            args,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_decl(&mut self, is_const: bool) -> Result<Stmt, String> {
        let start = self.peek().span;
        if is_const {
            self.eat(TokenKind::Const)?;
        } else {
            self.eat(TokenKind::Let)?;
        }
        let ty = if is_const && is_type_start(self.peek_kind()) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let mut names = vec![self.expect_ident()?];
        while self.eat_if(TokenKind::Comma) {
            names.push(self.expect_ident()?);
        }
        let value = if self.eat_if(TokenKind::Eq) {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Stmt::Decl {
            is_const,
            ty,
            names,
            value,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_typed_decl(&mut self) -> Result<Stmt, String> {
        let start = self.peek().span;
        let is_const = false;
        let ty = self.parse_type()?;
        let mut names = vec![self.expect_ident()?];
        while self.eat_if(TokenKind::Comma) {
            names.push(self.expect_ident()?);
        }
        let value = if self.eat_if(TokenKind::Eq) {
            Some(self.parse_expr()?)
        } else {
            None
        };
        Ok(Stmt::Decl {
            is_const,
            ty: Some(ty),
            names,
            value,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_if(&mut self) -> Result<Stmt, String> {
        let start = self.eat(TokenKind::If)?;
        self.eat(TokenKind::LParen)?;
        let cond = self.parse_expr()?;
        self.eat(TokenKind::RParen)?;
        let then_body = self.parse_block()?;
        let mut else_ifs = Vec::new();
        let mut else_body = None;
        while self.at(TokenKind::Else) {
            self.bump();
            if self.at(TokenKind::If) {
                self.bump();
                self.eat(TokenKind::LParen)?;
                let c = self.parse_expr()?;
                self.eat(TokenKind::RParen)?;
                let b = self.parse_block()?;
                else_ifs.push((c, b));
            } else {
                else_body = Some(self.parse_block()?);
                break;
            }
        }
        Ok(Stmt::If {
            cond,
            then_body,
            else_ifs,
            else_body,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_while(&mut self) -> Result<Stmt, String> {
        let start = self.eat(TokenKind::While)?;
        self.eat(TokenKind::LParen)?;
        let cond = self.parse_expr()?;
        self.eat(TokenKind::RParen)?;
        let body = self.parse_block()?;
        Ok(Stmt::While {
            cond,
            body,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_for(&mut self) -> Result<Stmt, String> {
        let start = self.eat(TokenKind::For)?;
        self.eat(TokenKind::LParen)?;
        // for (name in expr) vs for (int i = 0, cond, step)
        if self.at(TokenKind::Ident) && matches!(self.peek_n(1), TokenKind::In) {
            let name = self.expect_ident()?;
            self.eat(TokenKind::In)?;
            let iter = self.parse_expr()?;
            self.eat(TokenKind::RParen)?;
            let body = self.parse_block()?;
            return Ok(Stmt::ForIn {
                name,
                iter,
                body,
                span: start.merge(self.prev_span()),
            });
        }
        let init = self.parse_stmt()?;
        self.eat(TokenKind::Comma)?;
        let cond = self.parse_expr()?;
        self.eat(TokenKind::Comma)?;
        let step = self.parse_stmt()?;
        self.eat(TokenKind::RParen)?;
        let body = self.parse_block()?;
        Ok(Stmt::ForCount {
            init: Box::new(init),
            cond,
            step: Box::new(step),
            body,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_match(&mut self) -> Result<Stmt, String> {
        let start = self.eat(TokenKind::Match)?;
        self.eat(TokenKind::LParen)?;
        let expr = self.parse_expr()?;
        self.eat(TokenKind::RParen)?;
        self.eat(TokenKind::LBrace)?;
        let mut arms = Vec::new();
        let mut default = None;
        while !self.at(TokenKind::RBrace) && !self.at(TokenKind::Eof) {
            if self.eat_if(TokenKind::Default) {
                default = Some(self.parse_block()?);
            } else {
                let pat = self.parse_expr()?;
                let body = self.parse_block()?;
                arms.push((pat, body));
            }
        }
        self.eat(TokenKind::RBrace)?;
        Ok(Stmt::Match {
            expr,
            arms,
            default,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_goroutine(&mut self) -> Result<Stmt, String> {
        let start = self.eat(TokenKind::Goroutine)?;
        let body = self.parse_block()?;
        Ok(Stmt::GoroutineBlock {
            body,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_spawn(&mut self) -> Result<Stmt, String> {
        let start = self.eat(TokenKind::Spawn)?;
        if self.at(TokenKind::LBrace) {
            let body = self.parse_block()?;
            Ok(Stmt::SpawnBlock {
                body,
                span: start.merge(self.prev_span()),
            })
        } else {
            let expr = self.parse_expr()?;
            Ok(Stmt::SpawnExpr {
                expr,
                span: start.merge(self.prev_span()),
            })
        }
    }

    fn parse_asm(&mut self) -> Result<Stmt, String> {
        let start = self.eat(TokenKind::Asm)?;
        if !self.at(TokenKind::String) {
            return Err(self.error("expected inline assembly template string"));
        }
        let tok = self.bump();
        let template = tok.text;
        let mut clobbers = Vec::new();
        if self.eat_if(TokenKind::Colon) {
            loop {
                let name = self.expect_ident()?;
                clobbers.push(name);
                if !self.eat_if(TokenKind::Comma) {
                    break;
                }
            }
        }
        Ok(Stmt::Asm {
            template,
            clobbers,
            span: start.merge(self.prev_span()),
        })
    }

    fn parse_lock_stmt(&mut self) -> Result<Stmt, String> {
        let start = self.eat(TokenKind::Lock)?;
        if self.at(TokenKind::LParen) {
            self.bump();
            let name = self.expect_ident()?;
            self.eat(TokenKind::RParen)?;
            let body = self.parse_block()?;
            Ok(Stmt::LockBlock {
                name,
                body,
                span: start.merge(self.prev_span()),
            })
        } else {
            let name = self.expect_ident()?;
            Ok(Stmt::Decl {
                is_const: false,
                ty: Some(TypeAst::Named("lock".into())),
                names: vec![name],
                value: None,
                span: start.merge(self.prev_span()),
            })
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_otherwise()
    }

    fn parse_otherwise(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_or()?;
        while self.at(TokenKind::Otherwise) {
            self.bump();
            let rhs = self.parse_or()?;
            let span = lhs.span.merge(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op: BinOp::Otherwise,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_or(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_and()?;
        while self.at(TokenKind::Or) {
            self.bump();
            let rhs = self.parse_and()?;
            let span = lhs.span.merge(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op: BinOp::Or,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_bitor()?;
        while self.at(TokenKind::And) {
            self.bump();
            let rhs = self.parse_bitor()?;
            let span = lhs.span.merge(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op: BinOp::And,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_bitor(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_bitxor()?;
        while self.at(TokenKind::Pipe) {
            self.bump();
            let rhs = self.parse_bitxor()?;
            let span = lhs.span.merge(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op: BinOp::BitOr,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_bitxor(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_bitand()?;
        while self.at(TokenKind::Caret) {
            self.bump();
            let rhs = self.parse_bitand()?;
            let span = lhs.span.merge(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op: BinOp::BitXor,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_bitand(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_cmp()?;
        while self.at(TokenKind::Amp) {
            self.bump();
            let rhs = self.parse_cmp()?;
            let span = lhs.span.merge(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op: BinOp::BitAnd,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_cmp(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_shift()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::EqEq => BinOp::Eq,
                TokenKind::BangEq => BinOp::Ne,
                TokenKind::Lt => BinOp::Lt,
                TokenKind::Gt => BinOp::Gt,
                TokenKind::LtEq => BinOp::Le,
                TokenKind::GtEq => BinOp::Ge,
                TokenKind::In => BinOp::In,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_shift()?;
            let span = lhs.span.merge(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_shift(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_add()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Shl => BinOp::Shl,
                TokenKind::Shr => BinOp::Shr,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_add()?;
            let span = lhs.span.merge(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_add(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_mul()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Plus => BinOp::Add,
                TokenKind::Minus => BinOp::Sub,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_mul()?;
            let span = lhs.span.merge(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_mul(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_pow()?;
        loop {
            let op = match self.peek_kind() {
                TokenKind::Star => BinOp::Mul,
                TokenKind::Slash => BinOp::Div,
                TokenKind::Percent => BinOp::Mod,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_pow()?;
            let span = lhs.span.merge(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            };
        }
        Ok(lhs)
    }

    fn parse_pow(&mut self) -> Result<Expr, String> {
        let lhs = self.parse_unary()?;
        if self.at(TokenKind::StarStar) {
            self.bump();
            let rhs = self.parse_pow()?;
            let span = lhs.span.merge(rhs.span);
            return Ok(Expr {
                kind: ExprKind::Binary {
                    op: BinOp::Pow,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            });
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        if self.at(TokenKind::Minus) {
            let span = self.bump().span;
            let expr = self.parse_unary()?;
            return Ok(Expr {
                span: span.merge(expr.span),
                kind: ExprKind::Unary {
                    op: UnOp::Neg,
                    expr: Box::new(expr),
                },
            });
        }
        if self.at(TokenKind::Not) {
            let span = self.bump().span;
            let expr = self.parse_unary()?;
            return Ok(Expr {
                span: span.merge(expr.span),
                kind: ExprKind::Unary {
                    op: UnOp::Not,
                    expr: Box::new(expr),
                },
            });
        }
        if self.at(TokenKind::Tilde) {
            let span = self.bump().span;
            let expr = self.parse_unary()?;
            return Ok(Expr {
                span: span.merge(expr.span),
                kind: ExprKind::Unary {
                    op: UnOp::BitNot,
                    expr: Box::new(expr),
                },
            });
        }
        if self.at(TokenKind::Await) {
            let span = self.bump().span;
            let expr = self.parse_unary()?;
            return Ok(Expr {
                span: span.merge(expr.span),
                kind: ExprKind::Await(Box::new(expr)),
            });
        }
        if self.at(TokenKind::TryKw) {
            let span = self.bump().span;
            let expr = self.parse_unary()?;
            return Ok(Expr {
                span: span.merge(expr.span),
                kind: ExprKind::Try(Box::new(expr)),
            });
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, String> {
        let mut expr = self.parse_primary()?;
        loop {
            if self.at(TokenKind::LParen) {
                self.bump();
                let args = self.parse_arg_list()?;
                self.eat(TokenKind::RParen)?;
                let span = expr.span.merge(self.prev_span());
                expr = Expr {
                    kind: ExprKind::Call {
                        callee: Box::new(expr),
                        args,
                    },
                    span,
                };
            } else if self.at(TokenKind::LBracket) {
                self.bump();
                let index = self.parse_expr()?;
                self.eat(TokenKind::RBracket)?;
                let span = expr.span.merge(self.prev_span());
                expr = Expr {
                    kind: ExprKind::Index {
                        base: Box::new(expr),
                        index: Box::new(index),
                    },
                    span,
                };
            } else if self.at(TokenKind::Dot) {
                self.bump();
                let name = self.expect_ident()?;
                let span = expr.span.merge(self.prev_span());
                expr = Expr {
                    kind: ExprKind::Member {
                        base: Box::new(expr),
                        name,
                    },
                    span,
                };
            } else if self.at(TokenKind::Question) {
                let span = self.bump().span;
                expr = Expr {
                    span: expr.span.merge(span),
                    kind: ExprKind::OptionalChain(Box::new(expr)),
                };
            } else if self.at(TokenKind::BangBang) {
                let span = self.bump().span;
                expr = Expr {
                    span: expr.span.merge(span),
                    kind: ExprKind::ForceUnwrap(Box::new(expr)),
                };
            } else {
                break;
            }
        }
        Ok(expr)
    }

    fn parse_arg_list(&mut self) -> Result<Vec<Arg>, String> {
        let mut args = Vec::new();
        if self.at(TokenKind::RParen) {
            return Ok(args);
        }
        loop {
            if self.at(TokenKind::Ident) && matches!(self.peek_n(1), TokenKind::Colon) {
                let name = self.expect_ident()?;
                self.eat(TokenKind::Colon)?;
                let value = self.parse_expr()?;
                args.push(Arg::Named { name, value });
            } else {
                args.push(Arg::Pos(self.parse_expr()?));
            }
            if !self.eat_if(TokenKind::Comma) {
                break;
            }
        }
        Ok(args)
    }

    fn parse_primary(&mut self) -> Result<Expr, String> {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Int => {
                self.bump();
                Ok(Expr {
                    kind: ExprKind::Int(tok.int),
                    span: tok.span,
                })
            }
            TokenKind::Dec => {
                self.bump();
                Ok(Expr {
                    kind: ExprKind::Dec {
                        scaled: tok.int,
                        scale: tok.scale,
                    },
                    span: tok.span,
                })
            }
            TokenKind::FloatLit => {
                self.bump();
                Ok(Expr {
                    kind: ExprKind::Float(tok.float_val),
                    span: tok.span,
                })
            }
            TokenKind::True => {
                self.bump();
                Ok(Expr {
                    kind: ExprKind::Bool(true),
                    span: tok.span,
                })
            }
            TokenKind::False => {
                self.bump();
                Ok(Expr {
                    kind: ExprKind::Bool(false),
                    span: tok.span,
                })
            }
            TokenKind::None => {
                self.bump();
                Ok(Expr {
                    kind: ExprKind::None,
                    span: tok.span,
                })
            }
            TokenKind::SelfKw => {
                self.bump();
                Ok(Expr {
                    kind: ExprKind::Self_,
                    span: tok.span,
                })
            }
            TokenKind::Super => {
                let start = self.bump().span;
                self.eat(TokenKind::Dot)?;
                let method = self.expect_ident()?;
                self.eat(TokenKind::LParen)?;
                let args = self.parse_arg_list()?;
                self.eat(TokenKind::RParen)?;
                Ok(Expr {
                    kind: ExprKind::SuperCall { method, args },
                    span: start.merge(self.prev_span()),
                })
            }
            TokenKind::String => {
                self.bump();
                self.parse_string_expr(tok)
            }
            TokenKind::Ident => {
                let name = self.expect_ident()?;
                if name == "cast" && self.at(TokenKind::LParen) {
                    self.bump();
                    let expr = self.parse_expr()?;
                    self.eat(TokenKind::Comma)?;
                    let ty = self.parse_type()?;
                    self.eat(TokenKind::RParen)?;
                    return Ok(Expr {
                        span: tok.span.merge(self.prev_span()),
                        kind: ExprKind::Cast {
                            expr: Box::new(expr),
                            ty,
                        },
                    });
                }
                Ok(Expr {
                    kind: ExprKind::Ident(name),
                    span: tok.span,
                })
            }
            TokenKind::Fn => self.parse_lambda(tok.span),
            TokenKind::ErrorKw => {
                self.bump();
                Ok(Expr {
                    kind: ExprKind::Ident("error".into()),
                    span: tok.span,
                })
            }
            TokenKind::LParen => {
                self.bump();
                let first = self.parse_expr()?;
                if self.eat_if(TokenKind::Comma) {
                    let mut els = vec![first];
                    if !self.at(TokenKind::RParen) {
                        loop {
                            els.push(self.parse_expr()?);
                            if !self.eat_if(TokenKind::Comma) {
                                break;
                            }
                        }
                    }
                    self.eat(TokenKind::RParen)?;
                    Ok(Expr {
                        span: tok.span.merge(self.prev_span()),
                        kind: ExprKind::Tuple(els),
                    })
                } else {
                    self.eat(TokenKind::RParen)?;
                    Ok(first)
                }
            }
            TokenKind::LBracket => {
                self.bump();
                let mut els = Vec::new();
                if !self.at(TokenKind::RBracket) {
                    loop {
                        els.push(self.parse_expr()?);
                        if !self.eat_if(TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.eat(TokenKind::RBracket)?;
                Ok(Expr {
                    span: tok.span.merge(self.prev_span()),
                    kind: ExprKind::List(els),
                })
            }
            TokenKind::LBrace => {
                self.bump();
                let mut entries = Vec::new();
                if !self.at(TokenKind::RBrace) {
                    loop {
                        let k = self.parse_expr()?;
                        self.eat(TokenKind::Colon)?;
                        let v = self.parse_expr()?;
                        entries.push((k, v));
                        if !self.eat_if(TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.eat(TokenKind::RBrace)?;
                Ok(Expr {
                    span: tok.span.merge(self.prev_span()),
                    kind: ExprKind::Map(entries),
                })
            }
            _ => Err(self.error("expected expression")),
        }
    }

    fn parse_lambda(&mut self, start: Span) -> Result<Expr, String> {
        self.eat(TokenKind::Fn)?;
        self.eat(TokenKind::LParen)?;
        let mut params = Vec::new();
        if !self.at(TokenKind::RParen) {
            loop {
                params.push(self.parse_param()?);
                if !self.eat_if(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.eat(TokenKind::RParen)?;
        let ret = if self.eat_if(TokenKind::Arrow) {
            Some(self.parse_type()?)
        } else {
            None
        };
        let body = self.parse_block()?;
        Ok(Expr {
            span: start.merge(self.prev_span()),
            kind: ExprKind::Lambda { params, ret, body },
        })
    }

    fn parse_string_expr(&mut self, tok: Token) -> Result<Expr, String> {
        let s = tok.text;
        if !s.contains('{') {
            return Ok(Expr {
                kind: ExprKind::Str(s),
                span: tok.span,
            });
        }
        let mut parts = Vec::new();
        let mut buf = String::new();
        let chars: Vec<char> = s.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '{' {
                if !buf.is_empty() {
                    parts.push(InterpPart::Lit(std::mem::take(&mut buf)));
                }
                i += 1;
                let start = i;
                let mut depth = 1;
                while i < chars.len() && depth > 0 {
                    if chars[i] == '{' {
                        depth += 1;
                    } else if chars[i] == '}' {
                        depth -= 1;
                    }
                    if depth > 0 {
                        i += 1;
                    }
                }
                if depth != 0 {
                    return Err("unclosed interpolation".into());
                }
                let inner: String = chars[start..i].iter().collect();
                let inner_prefix: String = chars[..start].iter().collect();
                let base_offset = tok.span.start + 1 + inner_prefix.len() as u32;
                i += 1;
                let mut lexer = Lexer::new(self.file, &inner);
                let mut tokens = lexer.tokenize()?;
                for t in &mut tokens {
                    t.span.start += base_offset;
                    t.span.end += base_offset;
                }
                let mut ip = Parser {
                    tokens,
                    pos: 0,
                    file: self.file,
                };
                let inner_expr = ip.parse_expr()?;
                parts.push(InterpPart::Expr(inner_expr));
            } else {
                buf.push(chars[i]);
                i += 1;
            }
        }
        if !buf.is_empty() {
            parts.push(InterpPart::Lit(buf));
        }
        Ok(Expr {
            kind: ExprKind::Interpolate { parts },
            span: tok.span,
        })
    }

    fn assign_op(&self) -> Option<AssignOp> {
        match self.peek_kind() {
            TokenKind::Eq => Some(AssignOp::Eq),
            TokenKind::PlusEq => Some(AssignOp::PlusEq),
            TokenKind::MinusEq => Some(AssignOp::MinusEq),
            TokenKind::StarEq => Some(AssignOp::StarEq),
            TokenKind::SlashEq => Some(AssignOp::SlashEq),
            TokenKind::PercentEq => Some(AssignOp::PercentEq),
            TokenKind::AmpEq => Some(AssignOp::AmpEq),
            TokenKind::PipeEq => Some(AssignOp::PipeEq),
            TokenKind::CaretEq => Some(AssignOp::CaretEq),
            TokenKind::ShlEq => Some(AssignOp::ShlEq),
            TokenKind::ShrEq => Some(AssignOp::ShrEq),
            _ => None,
        }
    }

    fn starts_expr(&self) -> bool {
        matches!(
            self.peek_kind(),
            TokenKind::Ident
                | TokenKind::Int
                | TokenKind::Dec
                | TokenKind::String
                | TokenKind::True
                | TokenKind::False
                | TokenKind::None
                | TokenKind::SelfKw
                | TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::LBrace
                | TokenKind::Minus
                | TokenKind::Tilde
                | TokenKind::Not
                | TokenKind::Await
                | TokenKind::ErrorKw
                | TokenKind::TryKw
                | TokenKind::Fn
        )
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        match self.peek_kind() {
            TokenKind::Ident => Ok(self.bump().text),
            TokenKind::IntKw => {
                self.bump();
                Ok("int".into())
            }
            TokenKind::StrKw => {
                self.bump();
                Ok("str".into())
            }
            TokenKind::BoolKw => {
                self.bump();
                Ok("bool".into())
            }
            TokenKind::ErrorKw => {
                self.bump();
                Ok("error".into())
            }
            TokenKind::Default => {
                self.bump();
                Ok("default".into())
            }
            _ => Err(self.error("expected identifier")),
        }
    }

    fn expect_int(&mut self) -> Result<i64, String> {
        if self.at(TokenKind::Int) {
            Ok(self.bump().int)
        } else {
            Err(self.error("expected integer"))
        }
    }

    fn eat(&mut self, kind: TokenKind) -> Result<Span, String> {
        if self.at(kind) {
            Ok(self.bump().span)
        } else {
            Err(self.error(format!("expected {kind:?}, found {:?}", self.peek_kind())))
        }
    }

    fn eat_gt(&mut self) -> Result<Span, String> {
        if self.at(TokenKind::Gt) {
            Ok(self.bump().span)
        } else if self.at(TokenKind::Shr) {
            let cur = &mut self.tokens[self.pos];
            let orig_start = cur.span.start;
            let orig_file = cur.span.file;
            cur.span.start += 1;
            cur.text = ">".into();
            cur.kind = TokenKind::Gt;
            Ok(Span {
                file: orig_file,
                start: orig_start,
                end: orig_start + 1,
            })
        } else {
            Err(self.error(format!("expected '>', found {:?}", self.peek_kind())))
        }
    }

    fn eat_if(&mut self, kind: TokenKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn at(&self, kind: TokenKind) -> bool {
        self.peek_kind() == kind
    }

    fn peek(&self) -> &Token {
        self.tokens.get(self.pos).unwrap_or_else(|| self.tokens.last().unwrap())
    }

    fn peek_kind(&self) -> TokenKind {
        self.peek().kind
    }

    fn peek_n(&self, n: usize) -> TokenKind {
        self.tokens
            .get(self.pos + n)
            .map(|t| t.kind)
            .unwrap_or(TokenKind::Eof)
    }

    /// True when the next tokens form a variance annotation: `out T`, `in T`,
    /// or the `In` keyword followed by a type parameter name. `out` stays a
    /// plain identifier everywhere else, so `str out = ""` still parses.
    fn at_variance_marker(&self) -> bool {
        if self.at(TokenKind::In) {
            return matches!(
                self.tokens.get(self.pos + 1).map(|n| n.kind),
                Some(TokenKind::Ident)
            );
        }
        let t = self.peek();
        if t.kind != TokenKind::Ident {
            return false;
        }
        if t.text != "out" && t.text != "in" {
            return false;
        }
        // A type parameter name follows, so `out`/`in` here is the marker.
        matches!(
            self.tokens.get(self.pos + 1).map(|n| n.kind),
            Some(TokenKind::Ident)
        )
    }

    fn bump(&mut self) -> Token {
        let t = self.peek().clone();
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn prev_span(&self) -> Span {
        if self.pos == 0 {
            Span::dummy()
        } else {
            self.tokens[self.pos - 1].span
        }
    }

    fn error(&self, msg: impl Into<String>) -> String {
        let tok = self.peek();
        format!("{} (near '{}')", msg.into(), tok.text)
    }
}

fn is_type_start(k: TokenKind) -> bool {
    matches!(
        k,
        TokenKind::IntKw
            | TokenKind::I8Kw
            | TokenKind::I16Kw
            | TokenKind::I32Kw
            | TokenKind::U8Kw
            | TokenKind::U16Kw
            | TokenKind::U32Kw
            | TokenKind::U64Kw
            | TokenKind::FloatKw
            | TokenKind::StrKw
            | TokenKind::BoolKw
            | TokenKind::ByteKw
            | TokenKind::DecKw
            | TokenKind::ListKw
            | TokenKind::MapKw
            | TokenKind::Chan
            | TokenKind::RefKw
            | TokenKind::ErrorKw
            | TokenKind::AnyKw
            | TokenKind::Ptr
            | TokenKind::Mut
            | TokenKind::Ident
            | TokenKind::LParen
            | TokenKind::Fn
    )
}
