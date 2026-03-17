use std::collections::BTreeSet;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::transition::{Expr, Output, Property, State, TransitionSystem, VarDecl};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmitError {
    DuplicateName(String),
    MissingNextState(String),
    UnknownState(String),
    UnknownReference {
        expr_context: String,
        variable: String,
    },
}

impl Display for EmitError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateName(name) => write!(f, "duplicate declaration for `{name}`"),
            Self::MissingNextState(name) => write!(f, "missing next-state equation for `{name}`"),
            Self::UnknownState(name) => {
                write!(f, "next-state equation references unknown state `{name}`")
            }
            Self::UnknownReference {
                expr_context,
                variable,
            } => write!(
                f,
                "expression `{expr_context}` references unknown variable `{variable}`"
            ),
        }
    }
}

impl Error for EmitError {}

impl TransitionSystem {
    pub fn emit_lustre(&self) -> Result<String, EmitError> {
        validate_unique_names(&self.inputs, &self.states, &self.outputs, &self.properties)?;
        validate_next_states(self)?;
        validate_expression_references(self)?;

        let mut lines = Vec::new();
        lines.push(render_step_node_header(self));
        lines.push("let".to_string());
        for output in &self.outputs {
            lines.push(format!("  {} = {};", output.name, emit_expr(&output.expr)));
        }
        for next_state in &self.next_states {
            lines.push(format!(
                "  {} = {};",
                next_state_var_name(&next_state.state_name),
                emit_expr(&next_state.expr)
            ));
        }
        lines.push("tel".to_string());
        lines.push(String::new());

        lines.push(render_main_node_header(self));
        if !self.states.is_empty() {
            lines.push("var".to_string());
            for state in &self.states {
                lines.push(format!(
                    "  {}, {}: {};",
                    state_var_name(&state.name),
                    next_state_var_name(&state.name),
                    state.ty.as_lustre()
                ));
            }
        }
        lines.push("let".to_string());
        for state in &self.states {
            lines.push(format!(
                "  {} = {} -> pre({});",
                state_var_name(&state.name),
                emit_expr(&state.init),
                next_state_var_name(&state.name)
            ));
        }

        let step_outputs = self
            .outputs
            .iter()
            .map(|output| output.name.clone())
            .chain(
                self.states
                    .iter()
                    .map(|state| next_state_var_name(&state.name)),
            )
            .collect::<Vec<_>>()
            .join(", ");
        let step_args = self
            .inputs
            .iter()
            .map(|input| input.name.clone())
            .chain(self.states.iter().map(|state| state_var_name(&state.name)))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(format!(
            "  ({step_outputs}) = {}_step({step_args});",
            self.name
        ));
        for state in &self.states {
            lines.push(format!(
                "  {} = {};",
                state.name,
                state_var_name(&state.name)
            ));
        }
        for property in &self.properties {
            lines.push(format!(
                "  {} = {};",
                property.name,
                emit_expr(&property.expr)
            ));
            lines.push(format!("  --%PROPERTY {};", property.name));
        }
        lines.push("tel".to_string());

        Ok(lines.join("\n"))
    }
}

fn validate_unique_names(
    inputs: &[VarDecl],
    states: &[State],
    outputs: &[Output],
    properties: &[Property],
) -> Result<(), EmitError> {
    let mut names = BTreeSet::new();
    for name in inputs
        .iter()
        .map(|input| input.name.as_str())
        .chain(states.iter().map(|state| state.name.as_str()))
        .chain(outputs.iter().map(|output| output.name.as_str()))
        .chain(properties.iter().map(|property| property.name.as_str()))
    {
        if !names.insert(name) {
            return Err(EmitError::DuplicateName(name.to_string()));
        }
    }
    Ok(())
}

fn validate_next_states(system: &TransitionSystem) -> Result<(), EmitError> {
    let known_states = system
        .states
        .iter()
        .map(|state| state.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut seen = BTreeSet::new();
    for next_state in &system.next_states {
        if !known_states.contains(next_state.state_name.as_str()) {
            return Err(EmitError::UnknownState(next_state.state_name.clone()));
        }
        if !seen.insert(next_state.state_name.as_str()) {
            return Err(EmitError::DuplicateName(next_state.state_name.clone()));
        }
    }
    for state in &system.states {
        if !seen.contains(state.name.as_str()) {
            return Err(EmitError::MissingNextState(state.name.clone()));
        }
    }
    Ok(())
}

fn validate_expression_references(system: &TransitionSystem) -> Result<(), EmitError> {
    let step_scope = system
        .inputs
        .iter()
        .map(|input| input.name.as_str())
        .chain(system.states.iter().map(|state| state.name.as_str()))
        .collect::<BTreeSet<_>>();
    for output in &system.outputs {
        validate_expr(&output.expr, &step_scope, format!("output {}", output.name))?;
    }
    for next_state in &system.next_states {
        validate_expr(
            &next_state.expr,
            &step_scope,
            format!("next-state {}", next_state.state_name),
        )?;
    }

    let property_scope = system
        .inputs
        .iter()
        .map(|input| input.name.as_str())
        .chain(system.states.iter().map(|state| state.name.as_str()))
        .chain(system.outputs.iter().map(|output| output.name.as_str()))
        .collect::<BTreeSet<_>>();
    for property in &system.properties {
        validate_expr(
            &property.expr,
            &property_scope,
            format!("property {}", property.name),
        )?;
    }

    Ok(())
}

fn validate_expr(
    expr: &Expr,
    scope: &BTreeSet<&str>,
    expr_context: String,
) -> Result<(), EmitError> {
    for variable in expr.referenced_vars() {
        if !scope.contains(variable) {
            return Err(EmitError::UnknownReference {
                expr_context: expr_context.clone(),
                variable: variable.to_string(),
            });
        }
    }
    Ok(())
}

fn render_step_node_header(system: &TransitionSystem) -> String {
    let inputs = system
        .inputs
        .iter()
        .map(render_var_decl)
        .chain(
            system
                .states
                .iter()
                .map(|state| render_named_decl(state.name.as_str(), state.ty.as_lustre())),
        )
        .collect::<Vec<_>>()
        .join("; ");
    let returns = system
        .outputs
        .iter()
        .map(render_output_decl)
        .chain(system.states.iter().map(|state| {
            render_named_decl(&next_state_var_name(&state.name), state.ty.as_lustre())
        }))
        .collect::<Vec<_>>()
        .join("; ");
    format!("node {}_step({inputs}) returns ({returns});", system.name)
}

fn render_main_node_header(system: &TransitionSystem) -> String {
    let inputs = system
        .inputs
        .iter()
        .map(render_var_decl)
        .collect::<Vec<_>>()
        .join("; ");
    let returns = system
        .outputs
        .iter()
        .map(render_output_decl)
        .chain(
            system
                .states
                .iter()
                .map(|state| render_named_decl(state.name.as_str(), state.ty.as_lustre())),
        )
        .chain(
            system
                .properties
                .iter()
                .map(|property| render_named_decl(property.name.as_str(), "bool")),
        )
        .collect::<Vec<_>>()
        .join("; ");
    format!("node main({inputs}) returns ({returns});")
}

fn render_var_decl(decl: &VarDecl) -> String {
    render_named_decl(decl.name.as_str(), decl.ty.as_lustre())
}

fn render_output_decl(output: &Output) -> String {
    render_named_decl(output.name.as_str(), output.ty.as_lustre())
}

fn render_named_decl(name: &str, ty: &str) -> String {
    format!("{name}: {ty}")
}

fn emit_expr(expr: &Expr) -> String {
    match expr {
        Expr::Var(name) => name.clone(),
        Expr::Bool(true) => "true".to_string(),
        Expr::Bool(false) => "false".to_string(),
        Expr::Int(value) => value.to_string(),
        Expr::Not(inner) => format!("not ({})", emit_expr(inner)),
        Expr::Add(lhs, rhs) => emit_infix("+", lhs, rhs),
        Expr::Sub(lhs, rhs) => emit_infix("-", lhs, rhs),
        Expr::Mul(lhs, rhs) => emit_infix("*", lhs, rhs),
        Expr::Eq(lhs, rhs) => emit_infix("=", lhs, rhs),
        Expr::Lt(lhs, rhs) => emit_infix("<", lhs, rhs),
        Expr::Le(lhs, rhs) => emit_infix("<=", lhs, rhs),
        Expr::Gt(lhs, rhs) => emit_infix(">", lhs, rhs),
        Expr::Ge(lhs, rhs) => emit_infix(">=", lhs, rhs),
        Expr::And(lhs, rhs) => emit_infix("and", lhs, rhs),
        Expr::Or(lhs, rhs) => emit_infix("or", lhs, rhs),
        Expr::Ite {
            condition,
            then_expr,
            else_expr,
        } => format!(
            "if {} then {} else {}",
            emit_expr(condition),
            emit_expr(then_expr),
            emit_expr(else_expr)
        ),
    }
}

fn emit_infix(op: &str, lhs: &Expr, rhs: &Expr) -> String {
    format!("({} {} {})", emit_expr(lhs), op, emit_expr(rhs))
}

fn state_var_name(name: &str) -> String {
    format!("state_{name}")
}

fn next_state_var_name(name: &str) -> String {
    format!("next_{name}")
}

#[cfg(test)]
mod tests {
    use crate::transition::{Expr, Output, Property, State, TransitionSystem, Type, VarDecl};

    use super::EmitError;

    fn sample_system() -> TransitionSystem {
        TransitionSystem::new("accumulator")
            .with_input(VarDecl::new("i", Type::Int))
            .with_state(State::new("s", Type::Int, Expr::int(0)))
            .with_output(Output::new(
                "o",
                Type::Int,
                Expr::add(Expr::var("i"), Expr::var("s")),
            ))
            .with_next_state("s", Expr::add(Expr::var("s"), Expr::int(1)))
            .with_property(Property::new(
                "state_non_negative",
                Expr::ge(Expr::var("s"), Expr::int(0)),
            ))
    }

    #[test]
    fn emits_lustre_for_a_basic_recurrence() {
        let emitted = sample_system().emit_lustre().unwrap();

        assert!(
            emitted
                .contains("node accumulator_step(i: int; s: int) returns (o: int; next_s: int);")
        );
        assert!(emitted.contains("state_s = 0 -> pre(next_s);"));
        assert!(emitted.contains("(o, next_s) = accumulator_step(i, state_s);"));
        assert!(emitted.contains("--%PROPERTY state_non_negative;"));
    }

    #[test]
    fn rejects_unknown_references() {
        let system = TransitionSystem::new("broken")
            .with_state(State::new("s", Type::Int, Expr::int(0)))
            .with_output(Output::new("o", Type::Int, Expr::var("missing")))
            .with_next_state("s", Expr::var("s"));

        let error = system.emit_lustre().unwrap_err();
        assert_eq!(
            error,
            EmitError::UnknownReference {
                expr_context: "output o".to_string(),
                variable: "missing".to_string(),
            }
        );
    }

    #[test]
    fn requires_a_next_state_equation_per_state() {
        let system = TransitionSystem::new("broken")
            .with_state(State::new("s", Type::Int, Expr::int(0)))
            .with_output(Output::new("o", Type::Int, Expr::var("s")));

        let error = system.emit_lustre().unwrap_err();
        assert_eq!(error, EmitError::MissingNextState("s".to_string()));
    }
}
