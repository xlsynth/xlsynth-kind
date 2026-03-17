#![allow(dead_code)]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DemoExpectation {
    Valid,
    Invalid,
}

pub struct DemoSpec {
    pub slug: &'static str,
    pub description: &'static str,
    pub file_rel_path: &'static str,
    pub lhs_entry: &'static str,
    pub rhs_entry: &'static str,
    pub lhs_init: u128,
    pub rhs_init: u128,
    pub expectation: DemoExpectation,
}

const DEMOS: &[DemoSpec] = &[
    DemoSpec {
        slug: "add_vs_widening",
        description: "direct u8 accumulation versus widened u16 state accumulation with truncation",
        file_rel_path: "examples/demos/observational/add_vs_widening.x",
        lhs_entry: "f",
        rhs_entry: "g",
        lhs_init: 0,
        rhs_init: 0,
        expectation: DemoExpectation::Valid,
    },
    DemoSpec {
        slug: "delay_vs_widened_delay",
        description: "one-step input delay with u8 state versus widened u16 delayed state",
        file_rel_path: "examples/demos/observational/delay_vs_widened_delay.x",
        lhs_entry: "f",
        rhs_entry: "g",
        lhs_init: 0,
        rhs_init: 0,
        expectation: DemoExpectation::Valid,
    },
    DemoSpec {
        slug: "lsb_toggle_vs_counter",
        description: "u1 toggling state versus u8 counter whose low bit carries the same output trace",
        file_rel_path: "examples/demos/observational/lsb_toggle_vs_counter.x",
        lhs_entry: "f",
        rhs_entry: "g",
        lhs_init: 0,
        rhs_init: 0,
        expectation: DemoExpectation::Valid,
    },
    DemoSpec {
        slug: "shifted_input_vs_widened_shifted_input",
        description: "shifted u8 input versus widened u16 shift followed by truncation",
        file_rel_path: "examples/demos/observational/shifted_input_vs_widened_shifted_input.x",
        lhs_entry: "f",
        rhs_entry: "g",
        lhs_init: 0,
        rhs_init: 0,
        expectation: DemoExpectation::Valid,
    },
    DemoSpec {
        slug: "binary_vs_one_hot_latched_detector",
        description: "classic three-state detector with binary encoding versus one-hot encoding",
        file_rel_path: "examples/demos/observational/binary_vs_one_hot_latched_detector.x",
        lhs_entry: "f",
        rhs_entry: "g",
        lhs_init: 0,
        rhs_init: 1,
        expectation: DemoExpectation::Valid,
    },
    DemoSpec {
        slug: "unreachable_state_elimination",
        description: "parity FSM with an unreachable dead state versus the eliminated-state machine",
        file_rel_path: "examples/demos/observational/unreachable_state_elimination.x",
        lhs_entry: "f",
        rhs_entry: "g",
        lhs_init: 0,
        rhs_init: 0,
        expectation: DemoExpectation::Valid,
    },
    DemoSpec {
        slug: "equivalent_state_merge",
        description: "FSM with two equivalent states versus a merged-state implementation",
        file_rel_path: "examples/demos/observational/equivalent_state_merge.x",
        lhs_entry: "f",
        rhs_entry: "g",
        lhs_init: 0,
        rhs_init: 0,
        expectation: DemoExpectation::Valid,
    },
    DemoSpec {
        slug: "add_vs_offset_widening",
        description: "same recurrence shape as add_vs_widening, but rhs output is offset by one",
        file_rel_path: "examples/demos/observational/add_vs_offset_widening.x",
        lhs_entry: "f",
        rhs_entry: "g",
        lhs_init: 0,
        rhs_init: 0,
        expectation: DemoExpectation::Invalid,
    },
];

pub fn all_demos() -> &'static [DemoSpec] {
    DEMOS
}

pub fn find_demo(slug: &str) -> Option<&'static DemoSpec> {
    DEMOS.iter().find(|demo| demo.slug == slug)
}

pub fn valid_demos() -> impl Iterator<Item = &'static DemoSpec> {
    DEMOS
        .iter()
        .filter(|demo| matches!(demo.expectation, DemoExpectation::Valid))
}

pub fn invalid_demos() -> impl Iterator<Item = &'static DemoSpec> {
    DEMOS
        .iter()
        .filter(|demo| matches!(demo.expectation, DemoExpectation::Invalid))
}
