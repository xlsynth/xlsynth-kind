use std::error::Error;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

use xlsynth::{self, DslxConvertOptions, XlsynthError};
use xlsynth_pir::{ir as pir, ir_parser::Parser};

#[derive(Clone, Debug, Default)]
pub struct FrontendOptions {
    pub dslx_stdlib_path: Option<PathBuf>,
    pub additional_search_paths: Vec<PathBuf>,
    pub force_implicit_token_calling_convention: bool,
}

#[derive(Debug)]
pub struct FrontendArtifacts {
    pub module_path: PathBuf,
    pub ir_text: String,
    pub warnings: Vec<String>,
    pub pir_package: pir::Package,
}

impl FrontendArtifacts {
    pub fn top_fn(&self) -> Result<&pir::Fn, PipelineError> {
        self.pir_package
            .get_top_fn()
            .ok_or(PipelineError::NoTopFunction)
    }

    pub fn recurrence_signature(&self) -> Result<RecurrenceSignature, PipelineError> {
        infer_recurrence_signature(self.top_fn()?)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecurrenceSignature {
    pub input_name: String,
    pub input_ty: pir::Type,
    pub state_name: String,
    pub state_ty: pir::Type,
    pub output_ty: pir::Type,
    pub output_index: usize,
    pub next_state_index: usize,
}

#[derive(Debug)]
pub enum PipelineError {
    Io(std::io::Error),
    Xlsynth(XlsynthError),
    PirParse(String),
    NoTopFunction,
    UnexpectedRecurrenceShape(String),
    InvalidTop(String),
}

impl Display for PipelineError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "{err}"),
            Self::Xlsynth(err) => write!(f, "{err}"),
            Self::PirParse(err) => write!(f, "{err}"),
            Self::NoTopFunction => write!(f, "parsed PIR package does not contain a top function"),
            Self::UnexpectedRecurrenceShape(err) => write!(f, "{err}"),
            Self::InvalidTop(err) => write!(f, "{err}"),
        }
    }
}

impl Error for PipelineError {}

impl From<std::io::Error> for PipelineError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<XlsynthError> for PipelineError {
    fn from(value: XlsynthError) -> Self {
        Self::Xlsynth(value)
    }
}

pub fn compile_dslx_path(
    path: &Path,
    entry_name: Option<&str>,
    options: &FrontendOptions,
) -> Result<FrontendArtifacts, PipelineError> {
    let dslx = fs::read_to_string(path)?;
    compile_dslx_text(path, &dslx, entry_name, options)
}

pub fn compile_dslx_text(
    path: &Path,
    dslx: &str,
    entry_name: Option<&str>,
    options: &FrontendOptions,
) -> Result<FrontendArtifacts, PipelineError> {
    let additional_search_paths = options
        .additional_search_paths
        .iter()
        .map(PathBuf::as_path)
        .collect::<Vec<_>>();
    let convert_options = DslxConvertOptions {
        dslx_stdlib_path: options.dslx_stdlib_path.as_deref(),
        additional_search_paths,
        enable_warnings: None,
        disable_warnings: None,
        force_implicit_token_calling_convention: options.force_implicit_token_calling_convention,
    };
    let compiled = xlsynth::convert_dslx_to_ir_text(dslx, path, &convert_options)?;
    let top_name = if let Some(entry_name) = entry_name {
        let module_name = xlsynth::dslx_path_to_module_name(path)?;
        Some(xlsynth::mangle_dslx_name(module_name, entry_name)?)
    } else {
        None
    };
    parse_ir_text(path, &compiled.ir, top_name.as_deref()).map(|mut artifacts| {
        artifacts.warnings = compiled.warnings;
        artifacts
    })
}

pub fn parse_ir_text(
    path: &Path,
    ir_text: &str,
    top_name: Option<&str>,
) -> Result<FrontendArtifacts, PipelineError> {
    let mut parser = Parser::new(ir_text);
    let mut pir_package = parser
        .parse_package()
        .map_err(|err| PipelineError::PirParse(err.to_string()))?;

    if let Some(top_name) = top_name {
        pir_package
            .set_top_fn(top_name)
            .map_err(PipelineError::InvalidTop)?;
    }

    Ok(FrontendArtifacts {
        module_path: path.to_path_buf(),
        ir_text: ir_text.to_string(),
        warnings: Vec::new(),
        pir_package,
    })
}

pub fn infer_recurrence_signature(func: &pir::Fn) -> Result<RecurrenceSignature, PipelineError> {
    let [input_param, state_param] = func.params.as_slice() else {
        return Err(PipelineError::UnexpectedRecurrenceShape(format!(
            "expected top function `{}` to have exactly 2 parameters, got {}",
            func.name,
            func.params.len()
        )));
    };

    let pir::Type::Tuple(return_members) = &func.ret_ty else {
        return Err(PipelineError::UnexpectedRecurrenceShape(format!(
            "expected top function `{}` to return a 2-tuple, got {}",
            func.name, func.ret_ty
        )));
    };
    if return_members.len() != 2 {
        return Err(PipelineError::UnexpectedRecurrenceShape(format!(
            "expected top function `{}` to return a 2-tuple, got {} elements",
            func.name,
            return_members.len()
        )));
    }

    let first_ty = return_members[0].as_ref().clone();
    let second_ty = return_members[1].as_ref().clone();
    let (output_ty, output_index, next_state_index) = if second_ty == state_param.ty {
        (first_ty, 0, 1)
    } else if first_ty == state_param.ty {
        (second_ty, 1, 0)
    } else {
        return Err(PipelineError::UnexpectedRecurrenceShape(format!(
            "expected top function `{}` to return next-state type matching state parameter `{}` of type {}, got ({}, {})",
            func.name, state_param.name, state_param.ty, first_ty, second_ty
        )));
    };

    Ok(RecurrenceSignature {
        input_name: input_param.name.clone(),
        input_ty: input_param.ty.clone(),
        state_name: state_param.name.clone(),
        state_ty: state_param.ty.clone(),
        output_ty,
        output_index,
        next_state_index,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use xlsynth::{FnBuilder, IrPackage};
    use xlsynth_pir::ir as pir;

    use super::{
        FrontendOptions, PipelineError, RecurrenceSignature, compile_dslx_text, parse_ir_text,
    };

    #[test]
    fn compiles_dslx_and_extracts_the_recurrence_signature() {
        let dslx = r#"
pub fn step(i: u32, s: u32) -> (u32, u32) {
    let o = i + s;
    let next_s = s + u32:1;
    (o, next_s)
}
"#;
        let artifacts = compile_dslx_text(
            Path::new("accumulator.x"),
            dslx,
            Some("step"),
            &FrontendOptions::default(),
        )
        .unwrap();

        assert!(artifacts.top_fn().unwrap().name.contains("step"));
        assert_eq!(
            artifacts.recurrence_signature().unwrap(),
            RecurrenceSignature {
                input_name: "i".to_string(),
                input_ty: pir::Type::Bits(32),
                state_name: "s".to_string(),
                state_ty: pir::Type::Bits(32),
                output_ty: pir::Type::Bits(32),
                output_index: 0,
                next_state_index: 1,
            }
        );
    }

    #[test]
    fn accepts_functions_that_return_state_then_output() {
        let dslx = r#"
pub fn step(i: u32, s: u16) -> (u16, u32) {
    let o = i + (s as u32);
    let next_s = s + u16:1;
    (next_s, o)
}
"#;
        let artifacts = compile_dslx_text(
            Path::new("reversed_accumulator.x"),
            dslx,
            Some("step"),
            &FrontendOptions::default(),
        )
        .unwrap();

        assert_eq!(
            artifacts.recurrence_signature().unwrap(),
            RecurrenceSignature {
                input_name: "i".to_string(),
                input_ty: pir::Type::Bits(32),
                state_name: "s".to_string(),
                state_ty: pir::Type::Bits(16),
                output_ty: pir::Type::Bits(32),
                output_index: 1,
                next_state_index: 0,
            }
        );
    }

    #[test]
    fn rejects_functions_that_do_not_return_next_state() {
        let dslx = r#"
pub fn step(i: u32, s: u32) -> u32 {
    i + s
}
"#;
        let artifacts = compile_dslx_text(
            Path::new("not_a_recurrence.x"),
            dslx,
            Some("step"),
            &FrontendOptions::default(),
        )
        .unwrap();

        let error = artifacts.recurrence_signature().unwrap_err();
        assert!(matches!(error, PipelineError::UnexpectedRecurrenceShape(_)));
    }

    #[test]
    fn parses_ir_text_and_sets_top_function() {
        let mut package = IrPackage::new("fuzz_package").unwrap();
        let bits8 = package.get_bits_type(8);
        let mut builder = FnBuilder::new(&mut package, "fuzz_test", true);
        let i = builder.param("i", &bits8);
        let s = builder.param("s", &bits8);
        let o = builder.add(&i, &s, None);
        let ret = builder.tuple(&[&o, &s], None);
        let _ = builder.build_with_return_value(&ret);
        let artifacts = parse_ir_text(
            Path::new("generated.ir"),
            &package.to_string(),
            Some("fuzz_test"),
        )
        .unwrap();

        assert_eq!(artifacts.top_fn().unwrap().name, "fuzz_test");
    }
}
