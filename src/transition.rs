use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Bool,
    Int,
}

impl Type {
    pub fn as_lustre(&self) -> &'static str {
        match self {
            Self::Bool => "bool",
            Self::Int => "int",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VarDecl {
    pub name: String,
    pub ty: Type,
}

impl VarDecl {
    pub fn new(name: impl Into<String>, ty: Type) -> Self {
        Self {
            name: name.into(),
            ty,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct State {
    pub name: String,
    pub ty: Type,
    pub init: Expr,
}

impl State {
    pub fn new(name: impl Into<String>, ty: Type, init: Expr) -> Self {
        Self {
            name: name.into(),
            ty,
            init,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Output {
    pub name: String,
    pub ty: Type,
    pub expr: Expr,
}

impl Output {
    pub fn new(name: impl Into<String>, ty: Type, expr: Expr) -> Self {
        Self {
            name: name.into(),
            ty,
            expr,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub expr: Expr,
}

impl Property {
    pub fn new(name: impl Into<String>, expr: Expr) -> Self {
        Self {
            name: name.into(),
            expr,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Var(String),
    Bool(bool),
    Int(i64),
    Not(Box<Expr>),
    Add(Box<Expr>, Box<Expr>),
    Sub(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Eq(Box<Expr>, Box<Expr>),
    Lt(Box<Expr>, Box<Expr>),
    Le(Box<Expr>, Box<Expr>),
    Gt(Box<Expr>, Box<Expr>),
    Ge(Box<Expr>, Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Ite {
        condition: Box<Expr>,
        then_expr: Box<Expr>,
        else_expr: Box<Expr>,
    },
}

impl Expr {
    pub fn var(name: impl Into<String>) -> Self {
        Self::Var(name.into())
    }

    pub fn bool(value: bool) -> Self {
        Self::Bool(value)
    }

    pub fn int(value: i64) -> Self {
        Self::Int(value)
    }

    pub fn not(expr: Expr) -> Self {
        Self::Not(Box::new(expr))
    }

    pub fn add(lhs: Expr, rhs: Expr) -> Self {
        Self::Add(Box::new(lhs), Box::new(rhs))
    }

    pub fn sub(lhs: Expr, rhs: Expr) -> Self {
        Self::Sub(Box::new(lhs), Box::new(rhs))
    }

    pub fn mul(lhs: Expr, rhs: Expr) -> Self {
        Self::Mul(Box::new(lhs), Box::new(rhs))
    }

    pub fn eq(lhs: Expr, rhs: Expr) -> Self {
        Self::Eq(Box::new(lhs), Box::new(rhs))
    }

    pub fn lt(lhs: Expr, rhs: Expr) -> Self {
        Self::Lt(Box::new(lhs), Box::new(rhs))
    }

    pub fn le(lhs: Expr, rhs: Expr) -> Self {
        Self::Le(Box::new(lhs), Box::new(rhs))
    }

    pub fn gt(lhs: Expr, rhs: Expr) -> Self {
        Self::Gt(Box::new(lhs), Box::new(rhs))
    }

    pub fn ge(lhs: Expr, rhs: Expr) -> Self {
        Self::Ge(Box::new(lhs), Box::new(rhs))
    }

    pub fn and(lhs: Expr, rhs: Expr) -> Self {
        Self::And(Box::new(lhs), Box::new(rhs))
    }

    pub fn or(lhs: Expr, rhs: Expr) -> Self {
        Self::Or(Box::new(lhs), Box::new(rhs))
    }

    pub fn ite(condition: Expr, then_expr: Expr, else_expr: Expr) -> Self {
        Self::Ite {
            condition: Box::new(condition),
            then_expr: Box::new(then_expr),
            else_expr: Box::new(else_expr),
        }
    }

    pub fn referenced_vars(&self) -> BTreeSet<&str> {
        let mut vars = BTreeSet::new();
        self.collect_referenced_vars(&mut vars);
        vars
    }

    fn collect_referenced_vars<'a>(&'a self, vars: &mut BTreeSet<&'a str>) {
        match self {
            Self::Var(name) => {
                vars.insert(name);
            }
            Self::Bool(_) | Self::Int(_) => {}
            Self::Not(expr) => expr.collect_referenced_vars(vars),
            Self::Add(lhs, rhs)
            | Self::Sub(lhs, rhs)
            | Self::Mul(lhs, rhs)
            | Self::Eq(lhs, rhs)
            | Self::Lt(lhs, rhs)
            | Self::Le(lhs, rhs)
            | Self::Gt(lhs, rhs)
            | Self::Ge(lhs, rhs)
            | Self::And(lhs, rhs)
            | Self::Or(lhs, rhs) => {
                lhs.collect_referenced_vars(vars);
                rhs.collect_referenced_vars(vars);
            }
            Self::Ite {
                condition,
                then_expr,
                else_expr,
            } => {
                condition.collect_referenced_vars(vars);
                then_expr.collect_referenced_vars(vars);
                else_expr.collect_referenced_vars(vars);
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NextState {
    pub state_name: String,
    pub expr: Expr,
}

impl NextState {
    pub fn new(state_name: impl Into<String>, expr: Expr) -> Self {
        Self {
            state_name: state_name.into(),
            expr,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransitionSystem {
    pub name: String,
    pub inputs: Vec<VarDecl>,
    pub states: Vec<State>,
    pub outputs: Vec<Output>,
    pub next_states: Vec<NextState>,
    pub properties: Vec<Property>,
}

impl TransitionSystem {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            inputs: Vec::new(),
            states: Vec::new(),
            outputs: Vec::new(),
            next_states: Vec::new(),
            properties: Vec::new(),
        }
    }

    pub fn with_input(mut self, input: VarDecl) -> Self {
        self.inputs.push(input);
        self
    }

    pub fn with_state(mut self, state: State) -> Self {
        self.states.push(state);
        self
    }

    pub fn with_output(mut self, output: Output) -> Self {
        self.outputs.push(output);
        self
    }

    pub fn with_next_state(mut self, state_name: impl Into<String>, expr: Expr) -> Self {
        self.next_states.push(NextState::new(state_name, expr));
        self
    }

    pub fn with_property(mut self, property: Property) -> Self {
        self.properties.push(property);
        self
    }
}
