//! Recursive-descent parser for lingo v0.1.
//!
//! We hand-roll the parser to keep error messages obvious and the
//! grammar legible.  See `docs/GRAMMAR.bnf` in the repo root for the
//! shape we're targeting.

use crate::ast::*;
use crate::error::{LingoError, Span, Stage};
use crate::lexer::{Tok, Token};

pub fn parse(tokens: Vec<Token>) -> Result<Program, LingoError> {
    let mut p = Parser { tokens, pos: 0 };
    p.skip_newlines();
    let mut items = Vec::new();
    while !p.at(Tok::Eof) {
        items.push(p.item()?);
        p.skip_newlines();
    }
    Ok(Program { items })
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn peek_tok(&self) -> &Tok {
        &self.tokens[self.pos].tok
    }

    fn at(&self, t: Tok) -> bool {
        std::mem::discriminant(&self.tokens[self.pos].tok) == std::mem::discriminant(&t)
    }

    fn advance(&mut self) -> Token {
        let t = self.tokens[self.pos].clone();
        if !matches!(t.tok, Tok::Eof) {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, t: Tok) -> bool {
        if self.at(t) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, t: Tok, what: &str) -> Result<Token, LingoError> {
        if std::mem::discriminant(&self.peek().tok) == std::mem::discriminant(&t) {
            Ok(self.advance())
        } else {
            Err(LingoError::new(
                Stage::Parse,
                format!("expected {what}, got {:?}", self.peek().tok),
                self.peek().span,
            ))
        }
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek_tok(), Tok::Newline) {
            self.advance();
        }
    }

    // ---- items ----

    fn item(&mut self) -> Result<Item, LingoError> {
        match self.peek_tok() {
            Tok::Fn => Ok(Item::Fn(self.fn_decl()?)),
            Tok::Const => Ok(Item::Const(self.const_decl()?)),
            Tok::Struct => Ok(Item::Struct(self.struct_decl()?)),
            Tok::Enum => Ok(Item::Enum(self.enum_decl()?)),
            Tok::Impl => Ok(Item::Impl(self.impl_block()?)),
            other => Err(LingoError::new(
                Stage::Parse,
                format!(
                    "expected `fn`, `const`, `struct`, `enum`, or `impl` at top level, got {:?}",
                    other
                ),
                self.peek().span,
            )),
        }
    }

    fn struct_decl(&mut self) -> Result<StructDecl, LingoError> {
        let start = self.peek().span.start;
        self.expect(Tok::Struct, "`struct`")?;
        let name_tok = self.expect(Tok::Ident("".into()), "struct name")?;
        let name = match name_tok.tok {
            Tok::Ident(s) => s,
            _ => unreachable!(),
        };
        self.expect(Tok::Colon, "`:`")?;
        self.expect(Tok::Newline, "newline")?;
        self.skip_newlines();
        self.expect(Tok::Indent, "indented field block")?;
        let mut fields = Vec::new();
        while !self.at(Tok::Dedent) && !self.at(Tok::Eof) {
            self.skip_newlines();
            if self.at(Tok::Dedent) || self.at(Tok::Eof) {
                break;
            }
            let f_start = self.peek().span.start;
            let f_name_tok = self.expect(Tok::Ident("".into()), "field name")?;
            let f_name = match f_name_tok.tok {
                Tok::Ident(s) => s,
                _ => unreachable!(),
            };
            self.expect(Tok::Colon, "`:` after field name")?;
            let ty = self.type_ref()?;
            let f_end = ty.span.end;
            self.expect(Tok::Newline, "newline")?;
            fields.push(FieldDecl {
                name: f_name,
                ty,
                span: Span::new(f_start, f_end),
            });
            self.skip_newlines();
        }
        let end = self.peek().span.start;
        self.expect(Tok::Dedent, "dedent")?;
        Ok(StructDecl {
            name,
            fields,
            span: Span::new(start, end),
        })
    }

    fn enum_decl(&mut self) -> Result<EnumDecl, LingoError> {
        let start = self.peek().span.start;
        self.expect(Tok::Enum, "`enum`")?;
        let name_tok = self.expect(Tok::Ident("".into()), "enum name")?;
        let name = match name_tok.tok {
            Tok::Ident(s) => s,
            _ => unreachable!(),
        };
        self.expect(Tok::Colon, "`:`")?;
        self.expect(Tok::Newline, "newline")?;
        self.skip_newlines();
        self.expect(Tok::Indent, "indented variant block")?;
        let mut variants = Vec::new();
        while !self.at(Tok::Dedent) && !self.at(Tok::Eof) {
            self.skip_newlines();
            if self.at(Tok::Dedent) || self.at(Tok::Eof) {
                break;
            }
            let v_start = self.peek().span.start;
            let v_name_tok = self.expect(Tok::Ident("".into()), "variant name")?;
            let v_name = match v_name_tok.tok {
                Tok::Ident(s) => s,
                _ => unreachable!(),
            };
            let mut payload = Vec::new();
            if self.eat(Tok::LParen) {
                if !self.at(Tok::RParen) {
                    loop {
                        payload.push(self.type_ref()?);
                        if !self.eat(Tok::Comma) {
                            break;
                        }
                    }
                }
                self.expect(Tok::RParen, "`)`")?;
            }
            let v_end = self.peek().span.start;
            self.expect(Tok::Newline, "newline")?;
            variants.push(EnumVariant {
                name: v_name,
                payload,
                span: Span::new(v_start, v_end),
            });
            self.skip_newlines();
        }
        let end = self.peek().span.start;
        self.expect(Tok::Dedent, "dedent")?;
        Ok(EnumDecl {
            name,
            variants,
            span: Span::new(start, end),
        })
    }

    fn impl_block(&mut self) -> Result<ImplBlock, LingoError> {
        let start = self.peek().span.start;
        self.expect(Tok::Impl, "`impl`")?;
        let name_tok = self.expect(Tok::Ident("".into()), "type to impl")?;
        let target = match name_tok.tok {
            Tok::Ident(s) => s,
            _ => unreachable!(),
        };
        self.expect(Tok::Colon, "`:`")?;
        self.expect(Tok::Newline, "newline")?;
        self.skip_newlines();
        self.expect(Tok::Indent, "indented method block")?;
        let mut methods = Vec::new();
        while !self.at(Tok::Dedent) && !self.at(Tok::Eof) {
            self.skip_newlines();
            if self.at(Tok::Dedent) || self.at(Tok::Eof) {
                break;
            }
            methods.push(self.fn_decl()?);
            self.skip_newlines();
        }
        let end = self.peek().span.start;
        self.expect(Tok::Dedent, "dedent")?;
        Ok(ImplBlock {
            target,
            methods,
            span: Span::new(start, end),
        })
    }

    fn fn_decl(&mut self) -> Result<FnDecl, LingoError> {
        let start = self.peek().span.start;
        self.expect(Tok::Fn, "`fn`")?;
        let name_tok = self.expect(Tok::Ident("".into()), "function name")?;
        let name = match name_tok.tok {
            Tok::Ident(s) => s,
            _ => unreachable!(),
        };
        self.expect(Tok::LParen, "`(`")?;
        let mut params = Vec::new();
        if !self.at(Tok::RParen) {
            // optionally `self` as first parameter (in impl blocks)
            if self.at(Tok::Self_) {
                let s_tok = self.advance();
                params.push(Param {
                    name: "self".into(),
                    ty: TypeRef { name: "Self".into(), type_args: Vec::new(), span: s_tok.span },
                    span: s_tok.span,
                });
                if self.eat(Tok::Comma) {
                    loop {
                        params.push(self.param()?);
                        if !self.eat(Tok::Comma) {
                            break;
                        }
                    }
                }
            } else {
                loop {
                    params.push(self.param()?);
                    if !self.eat(Tok::Comma) {
                        break;
                    }
                }
            }
        }
        self.expect(Tok::RParen, "`)`")?;
        let return_type = if self.eat(Tok::Arrow) {
            Some(self.type_ref()?)
        } else {
            None
        };
        self.expect(Tok::Colon, "`:` to start function body")?;
        let body = self.block()?;
        let end = body.span.end;
        Ok(FnDecl {
            name,
            params,
            return_type,
            body,
            span: Span::new(start, end),
        })
    }

    fn const_decl(&mut self) -> Result<ConstDecl, LingoError> {
        let start = self.peek().span.start;
        self.expect(Tok::Const, "`const`")?;
        let name_tok = self.expect(Tok::Ident("".into()), "constant name")?;
        let name = match name_tok.tok {
            Tok::Ident(s) => s,
            _ => unreachable!(),
        };
        let ty = if self.eat(Tok::Colon) {
            Some(self.type_ref()?)
        } else {
            None
        };
        self.expect(Tok::Assign, "`=`")?;
        let value = self.expr()?;
        let end = value.span.end;
        self.expect(Tok::Newline, "newline")?;
        Ok(ConstDecl {
            name,
            ty,
            value,
            span: Span::new(start, end),
        })
    }

    fn param(&mut self) -> Result<Param, LingoError> {
        let name_tok = self.expect(Tok::Ident("".into()), "parameter name")?;
        let name = match name_tok.tok {
            Tok::Ident(s) => s,
            _ => unreachable!(),
        };
        self.expect(Tok::Colon, "`:` after parameter name")?;
        let ty = self.type_ref()?;
        let end = ty.span.end;
        Ok(Param {
            name,
            ty,
            span: Span::new(name_tok.span.start, end),
        })
    }

    fn type_ref(&mut self) -> Result<TypeRef, LingoError> {
        let t = self.expect(Tok::Ident("".into()), "type name")?;
        let name = match t.tok {
            Tok::Ident(s) => s,
            _ => unreachable!(),
        };
        let mut type_args = Vec::new();
        let mut end = t.span.end;
        if self.eat(Tok::LBracket) {
            if !self.at(Tok::RBracket) {
                loop {
                    type_args.push(self.type_ref()?);
                    if !self.eat(Tok::Comma) {
                        break;
                    }
                }
            }
            let close = self.expect(Tok::RBracket, "`]` to close type args")?;
            end = close.span.end;
        }
        Ok(TypeRef {
            name,
            type_args,
            span: Span::new(t.span.start, end),
        })
    }

    // ---- block & statements ----

    fn block(&mut self) -> Result<Block, LingoError> {
        let start = self.peek().span.start;
        self.expect(Tok::Newline, "newline before indented block")?;
        self.skip_newlines();
        self.expect(Tok::Indent, "indented block")?;
        let mut stmts = Vec::new();
        while !self.at(Tok::Dedent) && !self.at(Tok::Eof) {
            self.skip_newlines();
            if self.at(Tok::Dedent) || self.at(Tok::Eof) {
                break;
            }
            stmts.push(self.stmt()?);
            self.skip_newlines();
        }
        let end = self.peek().span.start;
        self.expect(Tok::Dedent, "dedent to close block")?;
        Ok(Block {
            stmts,
            span: Span::new(start, end),
        })
    }

    fn stmt(&mut self) -> Result<Stmt, LingoError> {
        match self.peek_tok() {
            Tok::Let => self.let_stmt(),
            Tok::Return => self.return_stmt(),
            Tok::If => self.if_stmt(),
            Tok::For => self.for_stmt(),
            Tok::Match => self.match_stmt(),
            Tok::Break => {
                let span = self.advance().span;
                self.expect(Tok::Newline, "newline")?;
                Ok(Stmt::Break(span))
            }
            Tok::Continue => {
                let span = self.advance().span;
                self.expect(Tok::Newline, "newline")?;
                Ok(Stmt::Continue(span))
            }
            _ => {
                // expr or assignment
                let expr = self.expr()?;
                if self.eat(Tok::Assign) {
                    let value = self.expr()?;
                    let end = value.span.end;
                    self.expect(Tok::Newline, "newline")?;
                    let target = match expr.kind {
                        ExprKind::Ident(s) => AssignTarget::Name(s),
                        ExprKind::Field(obj, name) => AssignTarget::Field(obj, name),
                        _ => {
                            return Err(LingoError::new(
                                Stage::Parse,
                                "left side of `=` must be a name or a field access",
                                expr.span,
                            ))
                        }
                    };
                    Ok(Stmt::Assign {
                        target,
                        value,
                        span: Span::new(expr.span.start, end),
                    })
                } else {
                    self.expect(Tok::Newline, "newline")?;
                    Ok(Stmt::Expr(expr))
                }
            }
        }
    }

    fn match_stmt(&mut self) -> Result<Stmt, LingoError> {
        let start = self.peek().span.start;
        self.expect(Tok::Match, "`match`")?;
        let scrutinee = self.expr()?;
        self.expect(Tok::Colon, "`:` after match scrutinee")?;
        self.expect(Tok::Newline, "newline")?;
        self.skip_newlines();
        self.expect(Tok::Indent, "indented match arms")?;
        let mut arms = Vec::new();
        while !self.at(Tok::Dedent) && !self.at(Tok::Eof) {
            self.skip_newlines();
            if self.at(Tok::Dedent) || self.at(Tok::Eof) {
                break;
            }
            arms.push(self.match_arm()?);
            self.skip_newlines();
        }
        let end = self.peek().span.start;
        self.expect(Tok::Dedent, "dedent")?;
        Ok(Stmt::Match {
            scrutinee,
            arms,
            span: Span::new(start, end),
        })
    }

    fn match_arm(&mut self) -> Result<MatchArm, LingoError> {
        let start = self.peek().span.start;
        let pattern = self.pattern()?;
        self.expect(Tok::Colon, "`:` after pattern")?;
        // arm body: either inline single statement on same line, or a block
        let body = if matches!(self.peek_tok(), Tok::Newline) {
            self.block()?
        } else {
            // inline single statement
            let s = self.inline_stmt()?;
            let b_span = match &s {
                Stmt::Expr(e) => e.span,
                Stmt::Return { span, .. } => *span,
                Stmt::Break(sp) | Stmt::Continue(sp) => *sp,
                _ => Span::new(start, start),
            };
            Block { stmts: vec![s], span: b_span }
        };
        let end = body.span.end;
        Ok(MatchArm {
            pattern,
            body,
            span: Span::new(start, end),
        })
    }

    fn inline_stmt(&mut self) -> Result<Stmt, LingoError> {
        // a single statement on the same line as `pattern:`
        match self.peek_tok() {
            Tok::Return => self.return_stmt(),
            Tok::Break => {
                let span = self.advance().span;
                self.expect(Tok::Newline, "newline")?;
                Ok(Stmt::Break(span))
            }
            Tok::Continue => {
                let span = self.advance().span;
                self.expect(Tok::Newline, "newline")?;
                Ok(Stmt::Continue(span))
            }
            _ => {
                let e = self.expr()?;
                self.expect(Tok::Newline, "newline")?;
                Ok(Stmt::Expr(e))
            }
        }
    }

    fn pattern(&mut self) -> Result<Pattern, LingoError> {
        let tok = self.peek().clone();
        match tok.tok {
            Tok::Ident(_) => {
                let first = self.advance();
                let first_name = match first.tok {
                    Tok::Ident(s) => s,
                    _ => unreachable!(),
                };
                let first_upper = first_name.chars().next().map_or(false, |c| c.is_ascii_uppercase());
                if self.eat(Tok::Dot) {
                    let v_tok = self.expect(Tok::Ident("".into()), "variant name after `.`")?;
                    let variant = match v_tok.tok {
                        Tok::Ident(s) => s,
                        _ => unreachable!(),
                    };
                    let mut sub = Vec::new();
                    if self.eat(Tok::LParen) {
                        if !self.at(Tok::RParen) {
                            loop {
                                sub.push(self.pattern()?);
                                if !self.eat(Tok::Comma) {
                                    break;
                                }
                            }
                        }
                        self.expect(Tok::RParen, "`)`")?;
                    }
                    let end = self.peek().span.start;
                    Ok(Pattern::Variant {
                        type_name: Some(first_name),
                        variant,
                        sub,
                        span: Span::new(first.span.start, end),
                    })
                } else if self.at(Tok::LParen) && !first_upper {
                    // bare variant call like `some(x)` (option/result)
                    self.advance();
                    let mut sub = Vec::new();
                    if !self.at(Tok::RParen) {
                        loop {
                            sub.push(self.pattern()?);
                            if !self.eat(Tok::Comma) {
                                break;
                            }
                        }
                    }
                    self.expect(Tok::RParen, "`)`")?;
                    let end = self.peek().span.start;
                    Ok(Pattern::Variant {
                        type_name: None,
                        variant: first_name,
                        sub,
                        span: Span::new(first.span.start, end),
                    })
                } else if first_name == "_" {
                    Ok(Pattern::Wildcard(first.span))
                } else if first_upper {
                    // a bare uppercase identifier in pattern position = variant w/ no payload
                    Ok(Pattern::Variant {
                        type_name: None,
                        variant: first_name,
                        sub: Vec::new(),
                        span: first.span,
                    })
                } else {
                    Ok(Pattern::Bind(first_name, first.span))
                }
            }
            Tok::None_ => {
                self.advance();
                Ok(Pattern::Variant {
                    type_name: None,
                    variant: "none".into(),
                    sub: Vec::new(),
                    span: tok.span,
                })
            }
            Tok::True => {
                self.advance();
                Ok(Pattern::Literal(PatLit::Bool(true), tok.span))
            }
            Tok::False => {
                self.advance();
                Ok(Pattern::Literal(PatLit::Bool(false), tok.span))
            }
            Tok::Int(n) => {
                self.advance();
                Ok(Pattern::Literal(PatLit::Int(n), tok.span))
            }
            Tok::Str(s) => {
                self.advance();
                Ok(Pattern::Literal(PatLit::Str(s), tok.span))
            }
            Tok::Minus => {
                self.advance();
                let inner = self.expect(Tok::Int(0), "integer after `-`")?;
                let n = match inner.tok {
                    Tok::Int(n) => -n,
                    _ => unreachable!(),
                };
                Ok(Pattern::Literal(PatLit::Int(n), Span::new(tok.span.start, inner.span.end)))
            }
            other => Err(LingoError::new(
                Stage::Parse,
                format!("expected a pattern, got {:?}", other),
                tok.span,
            )),
        }
    }

    fn let_stmt(&mut self) -> Result<Stmt, LingoError> {
        let start = self.peek().span.start;
        self.expect(Tok::Let, "`let`")?;
        let is_mut = self.eat(Tok::Mut);
        let name_tok = self.expect(Tok::Ident("".into()), "variable name")?;
        let name = match name_tok.tok {
            Tok::Ident(s) => s,
            _ => unreachable!(),
        };
        let ty = if self.eat(Tok::Colon) {
            Some(self.type_ref()?)
        } else {
            None
        };
        self.expect(Tok::Assign, "`=`")?;
        let value = self.expr()?;
        let end = value.span.end;
        self.expect(Tok::Newline, "newline")?;
        Ok(Stmt::Let {
            is_mut,
            name,
            ty,
            value,
            span: Span::new(start, end),
        })
    }

    fn return_stmt(&mut self) -> Result<Stmt, LingoError> {
        let start = self.peek().span.start;
        self.expect(Tok::Return, "`return`")?;
        let value = if matches!(self.peek_tok(), Tok::Newline) {
            None
        } else {
            Some(self.expr()?)
        };
        let end = value.as_ref().map(|e| e.span.end).unwrap_or(start);
        self.expect(Tok::Newline, "newline")?;
        Ok(Stmt::Return {
            value,
            span: Span::new(start, end),
        })
    }

    fn if_stmt(&mut self) -> Result<Stmt, LingoError> {
        let start = self.peek().span.start;
        self.expect(Tok::If, "`if`")?;
        let cond = self.expr()?;
        self.expect(Tok::Colon, "`:` after if condition")?;
        let block = self.block()?;
        let mut arms = vec![(cond, block)];
        let mut else_block = None;
        loop {
            self.skip_newlines();
            if self.eat(Tok::Elif) {
                let c = self.expr()?;
                self.expect(Tok::Colon, "`:` after elif condition")?;
                let b = self.block()?;
                arms.push((c, b));
            } else if self.eat(Tok::Else) {
                self.expect(Tok::Colon, "`:` after else")?;
                else_block = Some(self.block()?);
                break;
            } else {
                break;
            }
        }
        let end = else_block
            .as_ref()
            .map(|b| b.span.end)
            .unwrap_or_else(|| arms.last().unwrap().1.span.end);
        Ok(Stmt::If {
            arms,
            else_block,
            span: Span::new(start, end),
        })
    }

    fn for_stmt(&mut self) -> Result<Stmt, LingoError> {
        let start = self.peek().span.start;
        self.expect(Tok::For, "`for`")?;
        let var_tok = self.expect(Tok::Ident("".into()), "loop variable")?;
        let var = match var_tok.tok {
            Tok::Ident(s) => s,
            _ => unreachable!(),
        };
        self.expect(Tok::In, "`in`")?;
        let iter = self.expr()?;
        self.expect(Tok::Colon, "`:` after for clause")?;
        let body = self.block()?;
        let end = body.span.end;
        Ok(Stmt::For {
            var,
            iter,
            body,
            span: Span::new(start, end),
        })
    }

    // ---- expressions (precedence climbing) ----

    fn expr(&mut self) -> Result<Expr, LingoError> {
        self.or_expr()
    }

    fn or_expr(&mut self) -> Result<Expr, LingoError> {
        let mut left = self.and_expr()?;
        while self.eat(Tok::Or) {
            let right = self.and_expr()?;
            let span = Span::new(left.span.start, right.span.end);
            left = Expr {
                kind: ExprKind::Binary(BinOp::Or, Box::new(left), Box::new(right)),
                span,
            };
        }
        Ok(left)
    }

    fn and_expr(&mut self) -> Result<Expr, LingoError> {
        let mut left = self.not_expr()?;
        while self.eat(Tok::And) {
            let right = self.not_expr()?;
            let span = Span::new(left.span.start, right.span.end);
            left = Expr {
                kind: ExprKind::Binary(BinOp::And, Box::new(left), Box::new(right)),
                span,
            };
        }
        Ok(left)
    }

    fn not_expr(&mut self) -> Result<Expr, LingoError> {
        if self.at(Tok::Not) {
            let start = self.advance().span.start;
            let inner = self.not_expr()?;
            let span = Span::new(start, inner.span.end);
            Ok(Expr {
                kind: ExprKind::Unary(UnOp::Not, Box::new(inner)),
                span,
            })
        } else {
            self.cmp_expr()
        }
    }

    fn cmp_expr(&mut self) -> Result<Expr, LingoError> {
        let left = self.range_expr()?;
        let op = match self.peek_tok() {
            Tok::Eq => Some(BinOp::Eq),
            Tok::Ne => Some(BinOp::Ne),
            Tok::Lt => Some(BinOp::Lt),
            Tok::Le => Some(BinOp::Le),
            Tok::Gt => Some(BinOp::Gt),
            Tok::Ge => Some(BinOp::Ge),
            _ => None,
        };
        if let Some(op) = op {
            self.advance();
            let right = self.range_expr()?;
            let span = Span::new(left.span.start, right.span.end);
            Ok(Expr {
                kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                span,
            })
        } else {
            Ok(left)
        }
    }

    fn range_expr(&mut self) -> Result<Expr, LingoError> {
        let left = self.add_expr()?;
        if self.eat(Tok::DotDot) {
            let right = self.add_expr()?;
            let span = Span::new(left.span.start, right.span.end);
            Ok(Expr {
                kind: ExprKind::Range(Box::new(left), Box::new(right)),
                span,
            })
        } else {
            Ok(left)
        }
    }

    fn add_expr(&mut self) -> Result<Expr, LingoError> {
        let mut left = self.mul_expr()?;
        loop {
            let op = match self.peek_tok() {
                Tok::Plus => BinOp::Add,
                Tok::Minus => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.mul_expr()?;
            let span = Span::new(left.span.start, right.span.end);
            left = Expr {
                kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                span,
            };
        }
        Ok(left)
    }

    fn mul_expr(&mut self) -> Result<Expr, LingoError> {
        let mut left = self.unary_expr()?;
        loop {
            let op = match self.peek_tok() {
                Tok::Star => BinOp::Mul,
                Tok::Slash => BinOp::Div,
                Tok::Percent => BinOp::Mod,
                _ => break,
            };
            self.advance();
            let right = self.unary_expr()?;
            let span = Span::new(left.span.start, right.span.end);
            left = Expr {
                kind: ExprKind::Binary(op, Box::new(left), Box::new(right)),
                span,
            };
        }
        Ok(left)
    }

    fn unary_expr(&mut self) -> Result<Expr, LingoError> {
        if self.at(Tok::Minus) {
            let start = self.advance().span.start;
            let inner = self.unary_expr()?;
            let span = Span::new(start, inner.span.end);
            Ok(Expr {
                kind: ExprKind::Unary(UnOp::Neg, Box::new(inner)),
                span,
            })
        } else {
            self.pow_expr()
        }
    }

    fn pow_expr(&mut self) -> Result<Expr, LingoError> {
        let left = self.postfix()?;
        if self.eat(Tok::StarStar) {
            // right-associative
            let right = self.unary_expr()?;
            let span = Span::new(left.span.start, right.span.end);
            Ok(Expr {
                kind: ExprKind::Binary(BinOp::Pow, Box::new(left), Box::new(right)),
                span,
            })
        } else {
            Ok(left)
        }
    }

    fn postfix(&mut self) -> Result<Expr, LingoError> {
        let mut e = self.primary()?;
        loop {
            if self.eat(Tok::LParen) {
                let mut args = Vec::new();
                if !self.at(Tok::RParen) {
                    loop {
                        args.push(self.arg()?);
                        if !self.eat(Tok::Comma) {
                            break;
                        }
                    }
                }
                let end_tok = self.expect(Tok::RParen, "`)` to close call")?;
                let span = Span::new(e.span.start, end_tok.span.end);
                e = Expr {
                    kind: ExprKind::Call(Box::new(e), args),
                    span,
                };
            } else if self.eat(Tok::Dot) {
                let name_tok = self.expect(Tok::Ident("".into()), "field or method name after `.`")?;
                let name = match name_tok.tok {
                    Tok::Ident(s) => s,
                    _ => unreachable!(),
                };
                let span = Span::new(e.span.start, name_tok.span.end);
                e = Expr {
                    kind: ExprKind::Field(Box::new(e), name),
                    span,
                };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn arg(&mut self) -> Result<Arg, LingoError> {
        // arg ::= (IDENT ":")? expr
        // need lookahead: ident followed by colon = keyword arg
        let start = self.peek().span.start;
        if let Tok::Ident(_) = self.peek_tok() {
            if self.pos + 1 < self.tokens.len()
                && matches!(self.tokens[self.pos + 1].tok, Tok::Colon)
            {
                let name_tok = self.advance();
                let name = match name_tok.tok {
                    Tok::Ident(s) => s,
                    _ => unreachable!(),
                };
                self.advance(); // colon
                let value = self.expr()?;
                let end = value.span.end;
                return Ok(Arg {
                    name: Some(name),
                    value,
                    span: Span::new(start, end),
                });
            }
        }
        let value = self.expr()?;
        let span = value.span;
        Ok(Arg {
            name: None,
            value,
            span,
        })
    }

    fn primary(&mut self) -> Result<Expr, LingoError> {
        let tok = self.peek().clone();
        match tok.tok {
            Tok::Int(n) => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Int(n),
                    span: tok.span,
                })
            }
            Tok::Float(n) => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Float(n),
                    span: tok.span,
                })
            }
            Tok::Str(s) => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Str(s),
                    span: tok.span,
                })
            }
            Tok::True => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Bool(true),
                    span: tok.span,
                })
            }
            Tok::False => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Bool(false),
                    span: tok.span,
                })
            }
            Tok::None_ => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::None_,
                    span: tok.span,
                })
            }
            Tok::Ident(s) => {
                self.advance();
                // vec literal: `vec[a, b, c]`
                if s == "vec" && self.at(Tok::LBracket) {
                    self.advance(); // consume `[`
                    let mut items = Vec::new();
                    if !self.at(Tok::RBracket) {
                        loop {
                            items.push(self.expr()?);
                            if !self.eat(Tok::Comma) {
                                break;
                            }
                        }
                    }
                    let close = self.expect(Tok::RBracket, "`]` to close vec literal")?;
                    return Ok(Expr {
                        kind: ExprKind::VecLit(items),
                        span: Span::new(tok.span.start, close.span.end),
                    });
                }
                // struct literal: `Name{field: value, ...}` (only if Name starts uppercase)
                if self.at(Tok::LBrace)
                    && s.chars().next().map_or(false, |c| c.is_ascii_uppercase())
                {
                    self.advance(); // consume `{`
                    let mut fields = Vec::new();
                    if !self.at(Tok::RBrace) {
                        loop {
                            let fname_tok =
                                self.expect(Tok::Ident("".into()), "field name in struct literal")?;
                            let fname = match fname_tok.tok {
                                Tok::Ident(s) => s,
                                _ => unreachable!(),
                            };
                            self.expect(Tok::Colon, "`:` after field name")?;
                            let val = self.expr()?;
                            fields.push((fname, val));
                            if !self.eat(Tok::Comma) {
                                break;
                            }
                        }
                    }
                    let close = self.expect(Tok::RBrace, "`}` to close struct literal")?;
                    return Ok(Expr {
                        kind: ExprKind::StructLit { name: s, fields },
                        span: Span::new(tok.span.start, close.span.end),
                    });
                }
                Ok(Expr {
                    kind: ExprKind::Ident(s),
                    span: tok.span,
                })
            }
            Tok::Self_ => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::Self_,
                    span: tok.span,
                })
            }
            Tok::Print => {
                self.advance();
                Ok(Expr {
                    kind: ExprKind::PrintBuiltin,
                    span: tok.span,
                })
            }
            Tok::LParen => {
                self.advance();
                let e = self.expr()?;
                self.expect(Tok::RParen, "`)`")?;
                Ok(e)
            }
            other => Err(LingoError::new(
                Stage::Parse,
                format!("expected an expression, got {:?}", other),
                tok.span,
            )),
        }
    }
}
