#![no_main]

use std::path::Path;
use std::sync::OnceLock;

use arbitrary::Unstructured;
use libfuzzer_sys::fuzz_target;
use xlsynth::IrPackage;
use xlsynth_kind2::{EquivError, EquivOptions, EquivOutcome, prove_ir_text_step_equivalence};
use xlsynth_pir::ir::Type as PirType;
use xlsynth_pir::ir_fuzz::{generate_fuzz_sample_with_type, generate_ir_fn};

const TOP_NAME: &str = "fuzz_test";

fn fail_on_unknown() -> bool {
    std::env::var("XLSYNTH_KIND2_FAIL_ON_UNKNOWN")
        .map(|value| value != "0")
        .unwrap_or(false)
}

fn tools_available() -> bool {
    static TOOLS: OnceLock<bool> = OnceLock::new();
    *TOOLS.get_or_init(|| EquivOptions::default().tools_available())
}

fn recurrence_types(width: usize) -> (Vec<PirType>, PirType) {
    let bits = PirType::Bits(width);
    (
        vec![bits.clone(), bits.clone()],
        PirType::Tuple(vec![Box::new(bits.clone()), Box::new(bits)]),
    )
}

fuzz_target!(|data: &[u8]| {
    if !tools_available() {
        return;
    }

    let mut u = Unstructured::new(data);
    let width = match u.int_in_range::<u8>(1..=8) {
        Ok(width) => usize::from(width),
        Err(_) => return,
    };
    let (param_types, return_type) = recurrence_types(width);
    let sample = match generate_fuzz_sample_with_type(&mut u, &param_types, &return_type) {
        Ok(sample) => sample,
        Err(_) => return,
    };

    let mut package = match IrPackage::new("fuzz_package") {
        Ok(package) => package,
        Err(_) => return,
    };
    if generate_ir_fn(sample.ops, &mut package, Some(&return_type)).is_err() {
        return;
    }

    let ir_text = package.to_string();
    let result = prove_ir_text_step_equivalence(
        Path::new("fuzz_generated.ir"),
        &ir_text,
        Some(TOP_NAME),
        &EquivOptions::default(),
    );
    match result {
        Ok(check) if check.outcome == EquivOutcome::Equivalent => {}
        Ok(check) if matches!(check.outcome, EquivOutcome::Unknown(_)) => {
            if fail_on_unknown() {
                panic!(
                    "roundtrip equivalence returned unknown for generated IR:\ninput bytes: {:02x?}\n{ir_text}\noutcome: {:?}\n",
                    data, check.outcome
                );
            }
        }
        Ok(check) => {
            panic!(
                "roundtrip equivalence failed for generated IR:\ninput bytes: {:02x?}\n{ir_text}\noutcome: {:?}\n",
                data, check.outcome
            );
        }
        Err(EquivError::Export(_)) | Err(EquivError::Pipeline(_)) | Err(EquivError::Xlsynth(_)) => {
        }
        Err(err) => panic!(
            "unexpected equivalence failure for generated IR:\ninput bytes: {:02x?}\n{ir_text}\n{err}",
            data
        ),
    }
});
