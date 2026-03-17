use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProofPlan {
    pub path: PathBuf,
    pub main: MainProofSpec,
    pub aux: Vec<AuxProofSpec>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MainProofSpec {
    pub kind: MainProofKind,
    pub depends_on: Vec<String>,
    pub lhs_assumption_top: Option<String>,
    pub lhs_assumption_ir: Option<PathBuf>,
    pub rhs_assumption_top: Option<String>,
    pub rhs_assumption_ir: Option<PathBuf>,
    pub lhs_observation_top: Option<String>,
    pub lhs_observation_ir: Option<PathBuf>,
    pub rhs_observation_top: Option<String>,
    pub rhs_observation_ir: Option<PathBuf>,
    pub lhs_state_projection_top: Option<String>,
    pub lhs_state_projection_ir: Option<PathBuf>,
    pub rhs_state_projection_top: Option<String>,
    pub rhs_state_projection_ir: Option<PathBuf>,
    pub relation_top: Option<String>,
    pub relation_ir: Option<PathBuf>,
    pub decompose_relation: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MainProofKind {
    Observational,
    StateProjection,
    Relation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuxProofSpec {
    pub name: String,
    pub kind: AuxProofKind,
    pub depends_on: Vec<String>,
    pub lhs_assumption_top: Option<String>,
    pub lhs_assumption_ir: Option<PathBuf>,
    pub rhs_assumption_top: Option<String>,
    pub rhs_assumption_ir: Option<PathBuf>,
    pub lhs_top: Option<String>,
    pub lhs_ir: Option<PathBuf>,
    pub rhs_top: Option<String>,
    pub rhs_ir: Option<PathBuf>,
    pub top: Option<String>,
    pub ir: Option<PathBuf>,
    pub decompose_relation: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuxProofKind {
    ObservationEquiv,
    StateProjectionEquiv,
    StateProjectionRelation,
    StateRelation,
    Relation,
    LhsInvariant,
    RhsInvariant,
}

impl AuxProofKind {
    pub fn is_cross_copy(self) -> bool {
        matches!(
            self,
            Self::ObservationEquiv
                | Self::StateProjectionEquiv
                | Self::StateProjectionRelation
                | Self::StateRelation
                | Self::Relation
        )
    }

    pub fn is_single_side(self) -> bool {
        matches!(self, Self::LhsInvariant | Self::RhsInvariant)
    }
}

#[derive(Debug)]
pub enum ProofPlanError {
    Io(std::io::Error),
    Toml(toml::de::Error),
    MissingMain,
    DuplicateAuxName {
        name: String,
    },
    UnknownDependency {
        owner: String,
        dependency: String,
    },
    CyclicDependency {
        cycle_root: String,
    },
    InvalidMainSpec(String),
    InvalidAuxSpec {
        name: String,
        reason: String,
    },
}

impl Display for ProofPlanError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::Toml(err) => write!(f, "{err}"),
            Self::MissingMain => write!(f, "proof plan requires a [main] section"),
            Self::DuplicateAuxName { name } => write!(f, "duplicate aux entry `{name}`"),
            Self::UnknownDependency { owner, dependency } => {
                write!(f, "`{owner}` depends on unknown aux `{dependency}`")
            }
            Self::CyclicDependency { cycle_root } => {
                write!(f, "cyclic aux dependency involving `{cycle_root}`")
            }
            Self::InvalidMainSpec(reason) => write!(f, "invalid [main] spec: {reason}"),
            Self::InvalidAuxSpec { name, reason } => {
                write!(f, "invalid aux `{name}` spec: {reason}")
            }
        }
    }
}

impl std::error::Error for ProofPlanError {}

impl From<std::io::Error> for ProofPlanError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<toml::de::Error> for ProofPlanError {
    fn from(value: toml::de::Error) -> Self {
        Self::Toml(value)
    }
}

#[derive(Debug, Deserialize)]
struct ProofPlanFile {
    main: Option<RawMainProofSpec>,
    #[serde(default)]
    aux: Vec<RawAuxProofSpec>,
}

#[derive(Clone, Debug, Deserialize)]
struct RawMainProofSpec {
    kind: MainProofKind,
    #[serde(default)]
    depends_on: Vec<String>,
    lhs_assumption_top: Option<String>,
    lhs_assumption_ir: Option<PathBuf>,
    rhs_assumption_top: Option<String>,
    rhs_assumption_ir: Option<PathBuf>,
    lhs_observation_top: Option<String>,
    lhs_observation_ir: Option<PathBuf>,
    rhs_observation_top: Option<String>,
    rhs_observation_ir: Option<PathBuf>,
    lhs_state_projection_top: Option<String>,
    lhs_state_projection_ir: Option<PathBuf>,
    rhs_state_projection_top: Option<String>,
    rhs_state_projection_ir: Option<PathBuf>,
    relation_top: Option<String>,
    relation_ir: Option<PathBuf>,
    #[serde(default)]
    decompose_relation: bool,
}

#[derive(Clone, Debug, Deserialize)]
struct RawAuxProofSpec {
    name: String,
    kind: AuxProofKind,
    #[serde(default)]
    depends_on: Vec<String>,
    lhs_assumption_top: Option<String>,
    lhs_assumption_ir: Option<PathBuf>,
    rhs_assumption_top: Option<String>,
    rhs_assumption_ir: Option<PathBuf>,
    lhs_top: Option<String>,
    lhs_ir: Option<PathBuf>,
    rhs_top: Option<String>,
    rhs_ir: Option<PathBuf>,
    top: Option<String>,
    ir: Option<PathBuf>,
    #[serde(default)]
    decompose_relation: bool,
}

pub fn parse_proof_plan(path: &Path) -> Result<ProofPlan, ProofPlanError> {
    let text = fs::read_to_string(path)?;
    let raw: ProofPlanFile = toml::from_str(&text)?;
    let main = raw.main.ok_or(ProofPlanError::MissingMain)?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));
    let main = resolve_main_spec(base_dir, main)?;
    let aux = raw
        .aux
        .into_iter()
        .map(|raw_aux| resolve_aux_spec(base_dir, raw_aux))
        .collect::<Result<Vec<_>, _>>()?;
    validate_plan(path, &main, &aux)?;
    Ok(ProofPlan {
        path: path.to_path_buf(),
        main,
        aux,
    })
}

pub fn ordered_aux_specs<'a>(
    plan: &'a ProofPlan,
) -> Result<Vec<&'a AuxProofSpec>, ProofPlanError> {
    let by_name = plan
        .aux
        .iter()
        .map(|aux| (aux.name.as_str(), aux))
        .collect::<BTreeMap<_, _>>();
    let mut ordered = Vec::new();
    let mut visiting = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for aux in &plan.aux {
        visit_aux(aux, &by_name, &mut visiting, &mut visited, &mut ordered)?;
    }
    Ok(ordered)
}

fn visit_aux<'a>(
    aux: &'a AuxProofSpec,
    by_name: &BTreeMap<&'a str, &'a AuxProofSpec>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    ordered: &mut Vec<&'a AuxProofSpec>,
) -> Result<(), ProofPlanError> {
    if visited.contains(&aux.name) {
        return Ok(());
    }
    if !visiting.insert(aux.name.clone()) {
        return Err(ProofPlanError::CyclicDependency {
            cycle_root: aux.name.clone(),
        });
    }
    for dependency in &aux.depends_on {
        let dep_aux = by_name
            .get(dependency.as_str())
            .copied()
            .ok_or_else(|| ProofPlanError::UnknownDependency {
                owner: aux.name.clone(),
                dependency: dependency.clone(),
            })?;
        visit_aux(dep_aux, by_name, visiting, visited, ordered)?;
    }
    visiting.remove(&aux.name);
    visited.insert(aux.name.clone());
    ordered.push(aux);
    Ok(())
}

fn resolve_main_spec(base_dir: &Path, raw: RawMainProofSpec) -> Result<MainProofSpec, ProofPlanError> {
    let spec = MainProofSpec {
        kind: raw.kind,
        depends_on: raw.depends_on,
        lhs_assumption_top: raw.lhs_assumption_top,
        lhs_assumption_ir: resolve_path(base_dir, raw.lhs_assumption_ir),
        rhs_assumption_top: raw.rhs_assumption_top,
        rhs_assumption_ir: resolve_path(base_dir, raw.rhs_assumption_ir),
        lhs_observation_top: raw.lhs_observation_top,
        lhs_observation_ir: resolve_path(base_dir, raw.lhs_observation_ir),
        rhs_observation_top: raw.rhs_observation_top,
        rhs_observation_ir: resolve_path(base_dir, raw.rhs_observation_ir),
        lhs_state_projection_top: raw.lhs_state_projection_top,
        lhs_state_projection_ir: resolve_path(base_dir, raw.lhs_state_projection_ir),
        rhs_state_projection_top: raw.rhs_state_projection_top,
        rhs_state_projection_ir: resolve_path(base_dir, raw.rhs_state_projection_ir),
        relation_top: raw.relation_top,
        relation_ir: resolve_path(base_dir, raw.relation_ir),
        decompose_relation: raw.decompose_relation,
    };
    validate_main_kind(&spec)?;
    Ok(spec)
}

fn resolve_aux_spec(base_dir: &Path, raw: RawAuxProofSpec) -> Result<AuxProofSpec, ProofPlanError> {
    let spec = AuxProofSpec {
        name: raw.name,
        kind: raw.kind,
        depends_on: raw.depends_on,
        lhs_assumption_top: raw.lhs_assumption_top,
        lhs_assumption_ir: resolve_path(base_dir, raw.lhs_assumption_ir),
        rhs_assumption_top: raw.rhs_assumption_top,
        rhs_assumption_ir: resolve_path(base_dir, raw.rhs_assumption_ir),
        lhs_top: raw.lhs_top,
        lhs_ir: resolve_path(base_dir, raw.lhs_ir),
        rhs_top: raw.rhs_top,
        rhs_ir: resolve_path(base_dir, raw.rhs_ir),
        top: raw.top,
        ir: resolve_path(base_dir, raw.ir),
        decompose_relation: raw.decompose_relation,
    };
    validate_aux_kind(&spec)?;
    Ok(spec)
}

fn resolve_path(base_dir: &Path, path: Option<PathBuf>) -> Option<PathBuf> {
    path.map(|path| {
        if path.is_absolute() {
            path
        } else {
            base_dir.join(path)
        }
    })
}

fn validate_plan(
    path: &Path,
    main: &MainProofSpec,
    aux: &[AuxProofSpec],
) -> Result<(), ProofPlanError> {
    let mut names = BTreeSet::new();
    for aux_spec in aux {
        if !names.insert(aux_spec.name.clone()) {
            return Err(ProofPlanError::DuplicateAuxName {
                name: aux_spec.name.clone(),
            });
        }
    }
    let aux_names = aux
        .iter()
        .map(|aux| aux.name.as_str())
        .collect::<BTreeSet<_>>();
    for dependency in &main.depends_on {
        if !aux_names.contains(dependency.as_str()) {
            return Err(ProofPlanError::UnknownDependency {
                owner: format!("{}:[main]", path.display()),
                dependency: dependency.clone(),
            });
        }
    }
    for aux_spec in aux {
        for dependency in &aux_spec.depends_on {
            if !aux_names.contains(dependency.as_str()) {
                return Err(ProofPlanError::UnknownDependency {
                    owner: aux_spec.name.clone(),
                    dependency: dependency.clone(),
                });
            }
        }
    }
    let _ = ordered_aux_specs(&ProofPlan {
        path: path.to_path_buf(),
        main: main.clone(),
        aux: aux.to_vec(),
    })?;
    Ok(())
}

fn validate_main_kind(spec: &MainProofSpec) -> Result<(), ProofPlanError> {
    match spec.kind {
        MainProofKind::Observational => Ok(()),
        MainProofKind::StateProjection => {
            if spec.lhs_state_projection_top.is_none() || spec.rhs_state_projection_top.is_none() {
                return Err(ProofPlanError::InvalidMainSpec(
                    "state_projection main requires both lhs_state_projection_top and rhs_state_projection_top".to_string(),
                ));
            }
            Ok(())
        }
        MainProofKind::Relation => {
            if spec.relation_top.is_none() {
                return Err(ProofPlanError::InvalidMainSpec(
                    "relation main requires relation_top".to_string(),
                ));
            }
            Ok(())
        }
    }
}

fn validate_aux_kind(spec: &AuxProofSpec) -> Result<(), ProofPlanError> {
    let invalid = |reason: &str| {
        Err(ProofPlanError::InvalidAuxSpec {
            name: spec.name.clone(),
            reason: reason.to_string(),
        })
    };
    match spec.kind {
        AuxProofKind::ObservationEquiv
        | AuxProofKind::StateProjectionEquiv
        | AuxProofKind::StateProjectionRelation => {
            if spec.lhs_top.is_none() || spec.rhs_top.is_none() {
                return invalid("cross-copy aux requires both lhs_top and rhs_top");
            }
            if spec.top.is_some() {
                return invalid("cross-copy aux does not use top");
            }
            Ok(())
        }
        AuxProofKind::StateRelation => {
            if spec.top.is_some() || spec.lhs_top.is_some() || spec.rhs_top.is_some() {
                return invalid("state_relation aux does not use top/lhs_top/rhs_top");
            }
            Ok(())
        }
        AuxProofKind::Relation => {
            if spec.top.is_none() {
                return invalid("relation aux requires top");
            }
            if spec.lhs_top.is_some() || spec.rhs_top.is_some() {
                return invalid("relation aux does not use lhs_top/rhs_top");
            }
            Ok(())
        }
        AuxProofKind::LhsInvariant | AuxProofKind::RhsInvariant => {
            if spec.top.is_none() {
                return invalid("single-side invariant aux requires top");
            }
            if spec.lhs_top.is_some() || spec.rhs_top.is_some() {
                return invalid("single-side invariant aux does not use lhs_top/rhs_top");
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AuxProofKind, MainProofKind, ordered_aux_specs, parse_proof_plan};

    #[test]
    fn parses_plan_and_resolves_aux_order() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = dir.path().join("plan.toml");
        std::fs::write(
            &plan_path,
            r#"
[main]
kind = "observational"
depends_on = ["aux_reason_bits"]

[[aux]]
name = "aux_seed"
kind = "lhs_invariant"
top = "aux_seed"

[[aux]]
name = "aux_reason_bits"
kind = "observation_equiv"
lhs_top = "aux_reason_bits_lhs"
rhs_top = "aux_reason_bits_rhs"
depends_on = ["aux_seed"]
"#,
        )
        .unwrap();
        let plan = parse_proof_plan(&plan_path).unwrap();
        assert_eq!(plan.main.kind, MainProofKind::Observational);
        assert_eq!(plan.aux[0].kind, AuxProofKind::LhsInvariant);
        let ordered = ordered_aux_specs(&plan)
            .unwrap()
            .into_iter()
            .map(|aux| aux.name.clone())
            .collect::<Vec<_>>();
        assert_eq!(ordered, vec!["aux_seed", "aux_reason_bits"]);
    }

    #[test]
    fn rejects_unknown_dependency() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = dir.path().join("plan.toml");
        std::fs::write(
            &plan_path,
            r#"
[main]
kind = "relation"
relation_top = "rel_main"
depends_on = ["aux_missing"]
"#,
        )
        .unwrap();
        let err = parse_proof_plan(&plan_path).unwrap_err();
        assert!(err.to_string().contains("unknown aux"));
    }

    #[test]
    fn rejects_invalid_aux_shape() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = dir.path().join("plan.toml");
        std::fs::write(
            &plan_path,
            r#"
[main]
kind = "observational"

[[aux]]
name = "aux_bad"
kind = "observation_equiv"
top = "oops"
"#,
        )
        .unwrap();
        let err = parse_proof_plan(&plan_path).unwrap_err();
        assert!(err.to_string().contains("cross-copy aux requires both lhs_top and rhs_top"));
    }

    #[test]
    fn parses_state_projection_relation_aux_kind() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = dir.path().join("plan.toml");
        std::fs::write(
            &plan_path,
            r#"
[main]
kind = "observational"
depends_on = ["aux_state"]

[[aux]]
name = "aux_state"
kind = "state_projection_relation"
lhs_top = "lhs_abs"
rhs_top = "rhs_abs"
"#,
        )
        .unwrap();
        let plan = parse_proof_plan(&plan_path).unwrap();
        assert_eq!(plan.aux[0].kind, AuxProofKind::StateProjectionRelation);
    }

    #[test]
    fn parses_state_relation_aux_kind() {
        let dir = tempfile::tempdir().unwrap();
        let plan_path = dir.path().join("plan.toml");
        std::fs::write(
            &plan_path,
            r#"
[main]
kind = "observational"
depends_on = ["aux_state"]

[[aux]]
name = "aux_state"
kind = "state_relation"
"#,
        )
        .unwrap();
        let plan = parse_proof_plan(&plan_path).unwrap();
        assert_eq!(plan.aux[0].kind, AuxProofKind::StateRelation);
    }
}
