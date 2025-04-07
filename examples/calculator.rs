use std::fmt;

use reblessive::Stk;

#[derive(Debug)]
enum UnaryOperator {
    Neg,
    Pos,
}

#[derive(Eq, PartialEq, Debug)]
enum BinaryOperator {
    Pow,
    Mul,
    Div,
    Add,
    Sub,
}

#[derive(Debug)]
enum Expression {
    Number(f64),
    Covered(Box<Expression>),
    Binary {
        left: Box<Expression>,
        op: BinaryOperator,
        right: Box<Expression>,
    },
    Unary {
        op: UnaryOperator,
        expr: Box<Expression>,
    },
}

#[derive(Debug)]
pub enum Error {
    Parse,
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse => write!(f, "Failed to parse expression"),
        }
    }
}

fn is_number_char(v: u8) -> bool {
    v.is_ascii_digit() || matches!(v, b'.' | b'e' | b'E')
}

struct Buffer<'a>(&'a [u8]);

impl Iterator for Buffer<'_> {
    type Item = u8;

    fn next(&mut self) -> Option<Self::Item> {
        let (head, tail) = self.0.split_first()?;
        self.0 = tail;
        Some(*head)
    }
}

impl Buffer<'_> {
    pub fn get<I>(&self, index: I) -> Option<&I::Output>
    where
        I: std::slice::SliceIndex<[u8]>,
    {
        self.0.get(index)
    }
}

async fn parse(
    ctx: &mut Stk,
    bytes: &mut Buffer<'_>,
    binding_power: u8,
) -> Result<Expression, Error> {
    let peek = bytes.get(0).copied();
    let mut lhs = loop {
        match peek {
            Some(b'+') => {
                bytes.next();
                let expr = ctx.run(|ctx| parse(ctx, bytes, 7)).await?;
                break Expression::Unary {
                    op: UnaryOperator::Pos,
                    expr: Box::new(expr),
                };
            }
            Some(b'-') => {
                bytes.next();
                let expr = ctx.run(|ctx| parse(ctx, bytes, 7)).await?;
                break Expression::Unary {
                    op: UnaryOperator::Neg,
                    expr: Box::new(expr),
                };
            }
            Some(b'(') => {
                bytes.next();
                let expr = ctx.run(|ctx| parse(ctx, bytes, 0)).await?;
                let Some(b')') = bytes.next() else {
                    return Err(Error::Parse);
                };
                break Expression::Covered(Box::new(expr));
            }
            Some(x) if x.is_ascii_whitespace() => continue,
            Some(x) if is_number_char(x) => {
                let mut number = String::new();
                number.push(x as char);
                bytes.next();
                while bytes.get(0).copied().map(is_number_char).unwrap_or(false) {
                    let c = bytes.next().unwrap();
                    number.push(c as char);
                    if c.eq_ignore_ascii_case(&b'e') {
                        let n = bytes.get(0).copied();
                        if matches!(n, Some(b'-' | b'+')) {
                            bytes.next();
                            number.push(n.unwrap() as char);
                        }
                    }
                }
                let num = number.parse::<f64>().map_err(|_| Error::Parse)?;
                break Expression::Number(num);
            }
            _ => {
                return Err(Error::Parse);
            }
        };
    };

    loop {
        let (op, bp) = match bytes.get(0).copied() {
            Some(b'*') => {
                if let Some(b'*') = bytes.get(1) {
                    (BinaryOperator::Pow, (5, 6))
                } else {
                    (BinaryOperator::Mul, (3, 4))
                }
            }
            Some(b'/') => (BinaryOperator::Div, (3, 4)),
            Some(b'+') => (BinaryOperator::Add, (1, 2)),
            Some(b'-') => (BinaryOperator::Sub, (1, 2)),
            Some(x) if x.is_ascii_whitespace() => {
                continue;
            }
            _ => break,
        };

        if bp.0 < binding_power {
            break;
        }

        bytes.next();
        if op == BinaryOperator::Pow {
            bytes.next();
        }

        let rhs = ctx.run(|ctx| parse(ctx, bytes, bp.1)).await?;

        lhs = Expression::Binary {
            left: Box::new(lhs),
            op,
            right: Box::new(rhs),
        }
    }

    Ok(lhs)
}

async fn eval(ctx: &mut Stk, expr: &Expression) -> f64 {
    match expr {
        Expression::Number(x) => *x,
        Expression::Covered(ref x) => ctx.run(|ctx| eval(ctx, x)).await,
        Expression::Binary { left, op, right } => {
            let left = ctx.run(|ctx| eval(ctx, left)).await;
            let right = ctx.run(|ctx| eval(ctx, right)).await;
            match op {
                BinaryOperator::Pow => left.powf(right),
                BinaryOperator::Mul => left * right,
                BinaryOperator::Div => left / right,
                BinaryOperator::Add => left + right,
                BinaryOperator::Sub => left - right,
            }
        }
        Expression::Unary { op, expr } => {
            let expr = ctx.run(|ctx| eval(ctx, expr)).await;
            match op {
                UnaryOperator::Neg => -expr,
                UnaryOperator::Pos => expr,
            }
        }
    }
}

// A recursively defined simple calculater which can parse arbitrary depth expressions without
// ever overflowing the stack.
fn main() -> Result<(), Error> {
    let expr = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    if expr.is_empty() {
        return Ok(());
    }
    let mut stack = reblessive::Stack::new();
    let mut tokens = Buffer(expr.as_bytes());
    let expr = stack.enter(|ctx| parse(ctx, &mut tokens, 0)).finish()?;

    eprintln!("EXPRESSION: {:#?}", expr);

    println!("{}", stack.enter(|ctx| eval(ctx, &expr)).finish());

    Ok(())
}
