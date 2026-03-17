use std::env;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::path::Path;
use std::time::Instant;

use xlsynth_kind2::{
    Kind2CheckOutcome, Kind2Options, check_kind2_model, emit_relation_init_lustre,
    emit_relation_step_obligation_lustre, export_pir_fn_to_named_lustre_assumption_with_aggregates,
    export_pir_fn_to_named_lustre_relation_with_aggregates, export_pir_fn_to_named_lustre_step,
    extract_kind2_counterexample, extract_kind2_progress_lines, extract_kind2_runtime_line,
    parse_ir_text,
};

#[path = "demos/ir_observational/mod.rs"]
mod ir_observational_demos;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputFormat {
    Text,
    Csv,
    Json,
}

impl OutputFormat {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "text" => Ok(Self::Text),
            "csv" => Ok(Self::Csv),
            "json" => Ok(Self::Json),
            _ => Err(format!(
                "unsupported output format `{value}`; expected one of: text, csv, json"
            )),
        }
    }
}

#[derive(Debug)]
struct Options {
    list: bool,
    all: bool,
    show_progress: bool,
    format: OutputFormat,
    demo_slug: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PhaseStatus {
    Valid,
    Invalid,
    Unknown,
}

impl PhaseStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Invalid => "invalid",
            Self::Unknown => "unknown",
        }
    }
}

impl Display for PhaseStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PhaseResult {
    status: PhaseStatus,
    wall_millis: u128,
    runtime_line: Option<String>,
    counterexample: Option<String>,
    progress_lines: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DemoRunResult {
    slug: String,
    description: String,
    file_rel_path: String,
    step_entry: String,
    relation_file_rel_path: Option<String>,
    relation_entry: String,
    assumption_entry: Option<String>,
    lhs_state_width: usize,
    lhs_output_width: usize,
    init: PhaseResult,
    step: Option<PhaseResult>,
}

fn usage() -> ! {
    eprintln!(
        "usage: cargo run --example ir_observational_bench -- [--list] [--all] \\
         [--format text|csv|json] [--show-progress] [demo_slug]"
    );
    std::process::exit(2);
}

fn main() {
    let options = parse_args(env::args().skip(1).collect());
    if options.list {
        for demo in ir_observational_demos::all_demos() {
            println!("{}: {}", demo.slug, demo.description);
        }
        return;
    }

    let demos = select_demos(&options);
    let results = demos
        .into_iter()
        .map(|demo| run_demo(demo, options.show_progress))
        .collect::<Vec<_>>();

    match options.format {
        OutputFormat::Text => print_text_results(&results, options.show_progress),
        OutputFormat::Csv => print_csv_results(&results),
        OutputFormat::Json => print_json_results(&results),
    }

    let all_valid = results.iter().all(|result| {
        result.init.status == PhaseStatus::Valid
            && result
                .step
                .as_ref()
                .is_some_and(|step| step.status == PhaseStatus::Valid)
    });
    if !all_valid {
        std::process::exit(1);
    }
}

fn parse_args(args: Vec<String>) -> Options {
    let mut list = false;
    let mut all = false;
    let mut show_progress = false;
    let mut format = OutputFormat::Text;
    let mut demo_slug = None;
    let mut index = 0usize;
    while index < args.len() {
        match args[index].as_str() {
            "--help" | "-h" => usage(),
            "--list" => list = true,
            "--all" => all = true,
            "--show-progress" => show_progress = true,
            "--format" => {
                index += 1;
                let value = args.get(index).unwrap_or_else(|| usage());
                format = OutputFormat::parse(value).unwrap_or_else(|err| {
                    eprintln!("{err}");
                    usage();
                });
            }
            other if other.starts_with("--") => {
                eprintln!("unknown option `{other}`");
                usage();
            }
            other => {
                if demo_slug.replace(other.to_string()).is_some() {
                    eprintln!("expected at most one demo slug");
                    usage();
                }
            }
        }
        index += 1;
    }
    if all && demo_slug.is_some() {
        eprintln!("cannot combine `--all` with an explicit demo slug");
        usage();
    }
    Options {
        list,
        all,
        show_progress,
        format,
        demo_slug,
    }
}

fn select_demos(options: &Options) -> Vec<&'static ir_observational_demos::IrDemoSpec> {
    if options.all {
        return ir_observational_demos::all_demos().iter().collect();
    }
    if let Some(slug) = options.demo_slug.as_deref() {
        return vec![ir_observational_demos::find_demo(slug).unwrap_or_else(|| {
            eprintln!("unknown demo `{slug}`");
            usage();
        })];
    }
    vec![
        ir_observational_demos::all_demos()
            .first()
            .expect("expected at least one IR observational demo"),
    ]
}

fn run_demo(demo: &ir_observational_demos::IrDemoSpec, show_progress: bool) -> DemoRunResult {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let ir_path = repo_root.join(demo.file_rel_path);
    let ir_text = fs::read_to_string(&ir_path).expect("failed to read IR demo");
    let relation_ir_path = demo
        .relation_file_rel_path
        .map(|rel_path| repo_root.join(rel_path))
        .unwrap_or_else(|| ir_path.clone());
    let relation_ir_text = if relation_ir_path == ir_path {
        ir_text.clone()
    } else {
        fs::read_to_string(&relation_ir_path).expect("failed to read relation IR demo")
    };

    let step_artifacts =
        parse_ir_text(&ir_path, &ir_text, Some(demo.step_entry)).expect("failed to parse step");
    let relation_artifacts =
        parse_ir_text(&relation_ir_path, &relation_ir_text, Some(demo.relation_entry))
            .expect("failed to parse relation");
    let lhs = export_pir_fn_to_named_lustre_step(step_artifacts.top_fn().unwrap(), "step_impl_a")
        .expect("failed to export lhs step");
    let mut rhs = lhs.clone();
    rhs.node_name = "step_impl_b".to_string();
    rhs.state_only_node_name = "step_impl_b_state_only".to_string();
    rhs.lustre_text = rhs
        .lustre_text
        .replace("node step_impl_a(", "node step_impl_b(")
        .replace("node step_impl_a_state_only(", "node step_impl_b_state_only(");
    rhs.state_only_lustre_text = rhs
        .state_only_lustre_text
        .replace("node step_impl_a_state_only(", "node step_impl_b_state_only(");
    rhs.full_step_lustre_text = rhs
        .full_step_lustre_text
        .replace("node step_impl_a(", "node step_impl_b(");
    let assumption = demo.assumption_entry.map(|assumption_entry| {
        let assumption_artifacts = parse_ir_text(&ir_path, &ir_text, Some(assumption_entry))
            .expect("failed to parse assumption");
        export_pir_fn_to_named_lustre_assumption_with_aggregates(
            assumption_artifacts.top_fn().unwrap(),
            "assumption",
            lhs.input_aggregate.as_ref(),
            lhs.state_aggregate.as_ref(),
        )
        .expect("failed to export assumption")
    });
    let relation = export_pir_fn_to_named_lustre_relation_with_aggregates(
        relation_artifacts.top_fn().unwrap(),
        "relation",
        lhs.input_aggregate.as_ref(),
        lhs.state_aggregate.as_ref(),
        rhs.state_aggregate.as_ref(),
    )
    .expect("failed to export relation");

    let mut kind2_options = Kind2Options::default();
    if show_progress {
        kind2_options.verbosity = kind2_options.verbosity.max(2);
        kind2_options.live_output = true;
    }
    kind2_options.print_cex = true;
    let obligation_options = decomposed_relation_kind2_options(&kind2_options);

    let init_model = emit_relation_init_lustre(
        &lhs,
        &rhs,
        0,
        0,
        assumption.as_ref(),
        assumption.as_ref(),
        &relation,
    )
    .expect("failed to build relation init model");
    let init_start = Instant::now();
    let init_outcome = check_kind2_model(
        &init_model.lustre_text,
        init_model.top_node.as_str(),
        &obligation_options,
    )
    .expect("failed to run Kind2 on relation init");
    let init_wall = init_start.elapsed();
    let init = phase_result_from_outcome(&init_outcome, init_wall, show_progress);

    let step = if init.status == PhaseStatus::Valid {
        let step_model = emit_relation_step_obligation_lustre(
            &lhs,
            &rhs,
            assumption.as_ref(),
            assumption.as_ref(),
            &relation,
        )
        .expect("failed to build relation step model");
        let step_start = Instant::now();
        let step_outcome = check_kind2_model(
            &step_model.lustre_text,
            step_model.top_node.as_str(),
            &obligation_options,
        )
        .expect("failed to run Kind2 on relation step");
        let step_wall = step_start.elapsed();
        Some(phase_result_from_outcome(
            &step_outcome,
            step_wall,
            show_progress,
        ))
    } else {
        None
    };

    DemoRunResult {
        slug: demo.slug.to_string(),
        description: demo.description.to_string(),
        file_rel_path: demo.file_rel_path.to_string(),
        step_entry: demo.step_entry.to_string(),
        relation_file_rel_path: demo.relation_file_rel_path.map(str::to_string),
        relation_entry: demo.relation_entry.to_string(),
        assumption_entry: demo.assumption_entry.map(str::to_string),
        lhs_state_width: lhs.state_width,
        lhs_output_width: lhs.output_width,
        init,
        step,
    }
}

fn phase_result_from_outcome(
    outcome: &Kind2CheckOutcome,
    wall: std::time::Duration,
    show_progress: bool,
) -> PhaseResult {
    let summary = match outcome {
        Kind2CheckOutcome::Valid(summary)
        | Kind2CheckOutcome::Invalid(summary)
        | Kind2CheckOutcome::Unknown(summary) => summary,
    };
    let status = match outcome {
        Kind2CheckOutcome::Valid(_) => PhaseStatus::Valid,
        Kind2CheckOutcome::Invalid(_) => PhaseStatus::Invalid,
        Kind2CheckOutcome::Unknown(_) => PhaseStatus::Unknown,
    };
    PhaseResult {
        status,
        wall_millis: wall.as_millis(),
        runtime_line: extract_kind2_runtime_line(summary),
        counterexample: match status {
            PhaseStatus::Invalid => extract_kind2_counterexample(summary),
            _ => None,
        },
        progress_lines: if show_progress {
            extract_kind2_progress_lines(summary)
        } else {
            Vec::new()
        },
    }
}

fn print_text_results(results: &[DemoRunResult], show_progress: bool) {
    for (index, result) in results.iter().enumerate() {
        if index != 0 {
            println!();
        }
        println!("demo: {}", result.slug);
        println!("description: {}", result.description);
        println!("ir path: {}", result.file_rel_path);
        println!("step entry: {}", result.step_entry);
        if let Some(relation_file_rel_path) = result.relation_file_rel_path.as_deref() {
            println!("relation ir path: {relation_file_rel_path}");
        }
        println!("relation entry: {}", result.relation_entry);
        if let Some(assumption_entry) = result.assumption_entry.as_deref() {
            println!("assumption entry: {assumption_entry}");
        }
        println!("lhs state width: {}", result.lhs_state_width);
        println!("lhs output width: {}", result.lhs_output_width);
        print_phase_text("init relation", &result.init, show_progress);
        if let Some(step) = result.step.as_ref() {
            print_phase_text("step obligation", step, show_progress);
        } else {
            println!("step obligation result: skipped");
        }
    }
}

fn print_phase_text(label: &str, phase: &PhaseResult, show_progress: bool) {
    println!("{label} result: {}", phase.status);
    println!(
        "{label} wall time: {:.3}s",
        phase.wall_millis as f64 / 1000.0
    );
    if let Some(line) = phase.runtime_line.as_deref() {
        println!("{line}");
    }
    if let Some(counterexample) = phase.counterexample.as_deref() {
        println!("{label} counterexample:");
        println!("{counterexample}");
    }
    if show_progress && !phase.progress_lines.is_empty() {
        println!("{label} kind2 progress:");
        for line in &phase.progress_lines {
            println!("  {line}");
        }
    }
}

fn print_csv_results(results: &[DemoRunResult]) {
    println!(
        "slug,description,file_rel_path,step_entry,relation_file_rel_path,relation_entry,assumption_entry,lhs_state_width,lhs_output_width,init_result,init_wall_s,init_runtime_line,step_result,step_wall_s,step_runtime_line"
    );
    for result in results {
        let step_status = result
            .step
            .as_ref()
            .map(|phase| phase.status.as_str())
            .unwrap_or("");
        let step_wall = result
            .step
            .as_ref()
            .map(|phase| format!("{:.3}", phase.wall_millis as f64 / 1000.0))
            .unwrap_or_default();
        let step_runtime_line = result
            .step
            .as_ref()
            .and_then(|phase| phase.runtime_line.as_deref())
            .unwrap_or("");
        println!(
            "{},{},{},{},{},{},{},{},{},{:.3},{},{},{},{},{}",
            csv_escape(&result.slug),
            csv_escape(&result.description),
            csv_escape(&result.file_rel_path),
            csv_escape(&result.step_entry),
            csv_escape(result.relation_file_rel_path.as_deref().unwrap_or("")),
            csv_escape(&result.relation_entry),
            csv_escape(result.assumption_entry.as_deref().unwrap_or("")),
            result.lhs_state_width,
            result.lhs_output_width,
            result.init.status,
            result.init.wall_millis as f64 / 1000.0,
            csv_escape(result.init.runtime_line.as_deref().unwrap_or("")),
            step_status,
            step_wall,
            csv_escape(step_runtime_line),
        );
    }
}

fn print_json_results(results: &[DemoRunResult]) {
    println!("[");
    for (index, result) in results.iter().enumerate() {
        if index != 0 {
            println!(",");
        }
        print!("{}", demo_result_json(result));
    }
    println!("\n]");
}

fn demo_result_json(result: &DemoRunResult) -> String {
    format!(
        concat!(
            "  {{\n",
            "    \"slug\": {},\n",
            "    \"description\": {},\n",
            "    \"file_rel_path\": {},\n",
            "    \"step_entry\": {},\n",
            "    \"relation_file_rel_path\": {},\n",
            "    \"relation_entry\": {},\n",
            "    \"assumption_entry\": {},\n",
            "    \"lhs_state_width\": {},\n",
            "    \"lhs_output_width\": {},\n",
            "    \"init\": {},\n",
            "    \"step\": {}\n",
            "  }}"
        ),
        json_string(&result.slug),
        json_string(&result.description),
        json_string(&result.file_rel_path),
        json_string(&result.step_entry),
        json_option_string(result.relation_file_rel_path.as_deref()),
        json_string(&result.relation_entry),
        json_option_string(result.assumption_entry.as_deref()),
        result.lhs_state_width,
        result.lhs_output_width,
        phase_result_json(&result.init),
        match result.step.as_ref() {
            Some(step) => phase_result_json(step),
            None => "null".to_string(),
        },
    )
}

fn phase_result_json(phase: &PhaseResult) -> String {
    format!(
        concat!(
            "{{",
            "\"status\": {}, ",
            "\"wall_s\": {:.3}, ",
            "\"runtime_line\": {}, ",
            "\"counterexample\": {}, ",
            "\"progress_lines\": {}",
            "}}"
        ),
        json_string(phase.status.as_str()),
        phase.wall_millis as f64 / 1000.0,
        json_option_string(phase.runtime_line.as_deref()),
        json_option_string(phase.counterexample.as_deref()),
        json_string_array(&phase.progress_lines),
    )
}

fn json_string_array(values: &[String]) -> String {
    let inner = values
        .iter()
        .map(|value| json_string(value))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{inner}]")
}

fn json_option_string(value: Option<&str>) -> String {
    value.map(json_string).unwrap_or_else(|| "null".to_string())
}

fn json_string(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            '\u{08}' => escaped.push_str("\\b"),
            '\u{0C}' => escaped.push_str("\\f"),
            ch if ch.is_control() => escaped.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => escaped.push(ch),
        }
    }
    escaped.push('"');
    escaped
}

fn csv_escape(value: &str) -> String {
    if value
        .chars()
        .any(|ch| matches!(ch, ',' | '"' | '\n' | '\r'))
    {
        let escaped = value.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        value.to_string()
    }
}

fn decomposed_relation_kind2_options(base: &Kind2Options) -> Kind2Options {
    let mut options = base.clone();
    options.extra_args.extend(
        [
            "--enable",
            "BMC",
            "--enable",
            "IND",
            "--disable",
            "IND2",
            "--disable",
            "IC3QE",
            "--disable",
            "IC3IA",
            "--disable",
            "INVGEN",
            "--disable",
            "INVGENOS",
            "--disable",
            "INVGENINT",
            "--disable",
            "INVGENINTOS",
            "--disable",
            "INVGENMACH",
            "--disable",
            "INVGENMACHOS",
            "--disable",
            "INVGENREAL",
            "--disable",
            "INVGENREALOS",
            "--check_subproperties",
            "false",
        ]
        .into_iter()
        .map(str::to_string),
    );
    options
}

#[cfg(test)]
mod tests {
    use super::{
        DemoRunResult, PhaseResult, PhaseStatus, csv_escape, demo_result_json, parse_args,
    };

    fn sample_phase(status: PhaseStatus) -> PhaseResult {
        PhaseResult {
            status,
            wall_millis: 1234,
            runtime_line: Some("Runtime: 1.234s".to_string()),
            counterexample: None,
            progress_lines: vec!["k=1".to_string()],
        }
    }

    fn sample_result() -> DemoRunResult {
        DemoRunResult {
            slug: "batch_contract".to_string(),
            description: "batch countdown with a stable-annotation assumption".to_string(),
            file_rel_path: "examples/demos/ir_observational/batch_contract.ir".to_string(),
            step_entry: "step".to_string(),
            relation_file_rel_path: None,
            relation_entry: "rel".to_string(),
            assumption_entry: Some("env".to_string()),
            lhs_state_width: 7,
            lhs_output_width: 2,
            init: sample_phase(PhaseStatus::Valid),
            step: Some(sample_phase(PhaseStatus::Unknown)),
        }
    }

    #[test]
    fn parses_all_and_format_flags() {
        let options = parse_args(vec![
            "--all".to_string(),
            "--format".to_string(),
            "csv".to_string(),
        ]);
        assert!(options.all);
        assert_eq!(options.demo_slug, None);
        assert!(matches!(options.format, super::OutputFormat::Csv));
    }

    #[test]
    fn csv_escape_quotes_fields_when_needed() {
        assert_eq!(csv_escape("plain"), "plain");
        assert_eq!(csv_escape("a,b"), "\"a,b\"");
        assert_eq!(csv_escape("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn json_render_includes_phase_fields() {
        let json = demo_result_json(&sample_result());
        assert!(json.contains("\"slug\": \"batch_contract\""));
        assert!(json.contains("\"init\": {\"status\": \"valid\""));
        assert!(json.contains("\"step\": {\"status\": \"unknown\""));
        assert!(json.contains("\"relation_file_rel_path\": null"));
        assert!(json.contains("\"assumption_entry\": \"env\""));
    }
}
