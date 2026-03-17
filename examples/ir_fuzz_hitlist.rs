use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use arbitrary::Unstructured;
use xlsynth::IrPackage;
use xlsynth_kind2::{export_pir_fn_to_lustre_step, parse_ir_text};
use xlsynth_pir::ir::{Fn as PirFn, NodePayload, Type as PirType};
use xlsynth_pir::ir_fuzz::{generate_fuzz_sample_with_type, generate_ir_fn};

fn recurrence_types(width: usize) -> (Vec<PirType>, PirType) {
    let bits = PirType::Bits(width);
    (
        vec![bits.clone(), bits.clone()],
        PirType::Tuple(vec![Box::new(bits.clone()), Box::new(bits)]),
    )
}

fn sample_bytes(seed: u64, byte_count: usize) -> Vec<u8> {
    let mut state = seed
        .wrapping_mul(0x9e37_79b9_7f4a_7c15)
        .wrapping_add(0x94d0_49bb_1331_11eb);
    let mut out = Vec::with_capacity(byte_count);
    while out.len() < byte_count {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        state = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
        out.extend_from_slice(&state.to_le_bytes());
    }
    out.truncate(byte_count);
    out
}

fn collect_live_ops(func: &PirFn) -> BTreeSet<String> {
    let mut live = BTreeSet::new();
    let Some(ret_node_ref) = func.ret_node_ref else {
        return live;
    };
    let mut stack = vec![ret_node_ref];
    let mut seen = BTreeSet::new();
    while let Some(node_ref) = stack.pop() {
        if !seen.insert(node_ref.index) {
            continue;
        }
        let node = func.get_node(node_ref);
        live.insert(node.payload.get_operator().to_string());
        match &node.payload {
            NodePayload::Tuple(members)
            | NodePayload::Array(members)
            | NodePayload::AfterAll(members)
            | NodePayload::Nary(_, members) => stack.extend(members.iter().copied()),
            NodePayload::TupleIndex { tuple, .. }
            | NodePayload::Unop(_, tuple)
            | NodePayload::SignExt { arg: tuple, .. }
            | NodePayload::ZeroExt { arg: tuple, .. }
            | NodePayload::OneHot { arg: tuple, .. }
            | NodePayload::Encode { arg: tuple }
            | NodePayload::Decode { arg: tuple, .. } => stack.push(*tuple),
            NodePayload::Binop(_, lhs, rhs) => {
                stack.push(*lhs);
                stack.push(*rhs);
            }
            NodePayload::BitSlice { arg, .. } => stack.push(*arg),
            NodePayload::DynamicBitSlice { arg, start, .. } => {
                stack.push(*arg);
                stack.push(*start);
            }
            NodePayload::BitSliceUpdate {
                arg,
                start,
                update_value,
            } => {
                stack.push(*arg);
                stack.push(*start);
                stack.push(*update_value);
            }
            NodePayload::ExtCarryOut { lhs, rhs, c_in } => {
                stack.push(*lhs);
                stack.push(*rhs);
                stack.push(*c_in);
            }
            NodePayload::PrioritySel {
                selector,
                cases,
                default,
            } => {
                stack.push(*selector);
                stack.extend(cases.iter().copied());
                if let Some(default) = default {
                    stack.push(*default);
                }
            }
            NodePayload::OneHotSel { selector, cases } => {
                stack.push(*selector);
                stack.extend(cases.iter().copied());
            }
            NodePayload::Sel {
                selector,
                cases,
                default,
            } => {
                stack.push(*selector);
                stack.extend(cases.iter().copied());
                if let Some(default) = default {
                    stack.push(*default);
                }
            }
            NodePayload::ArraySlice { array, start, .. } => {
                stack.push(*array);
                stack.push(*start);
            }
            NodePayload::ArrayUpdate {
                array,
                value,
                indices,
                ..
            } => {
                stack.push(*array);
                stack.push(*value);
                stack.extend(indices.iter().copied());
            }
            NodePayload::ArrayIndex { array, indices, .. } => {
                stack.push(*array);
                stack.extend(indices.iter().copied());
            }
            NodePayload::Invoke { operands, .. } => stack.extend(operands.iter().copied()),
            NodePayload::CountedFor {
                init,
                invariant_args,
                ..
            } => {
                stack.push(*init);
                stack.extend(invariant_args.iter().copied());
            }
            NodePayload::Assert {
                token, activate, ..
            } => {
                stack.push(*token);
                stack.push(*activate);
            }
            NodePayload::Trace {
                token,
                activated,
                operands,
                ..
            } => {
                stack.push(*token);
                stack.push(*activated);
                stack.extend(operands.iter().copied());
            }
            NodePayload::InstantiationInput { arg, .. }
            | NodePayload::RegisterWrite { arg, .. }
            | NodePayload::Cover { predicate: arg, .. } => stack.push(*arg),
            NodePayload::Nil
            | NodePayload::GetParam(_)
            | NodePayload::Literal(_)
            | NodePayload::ExtPrioEncode { .. }
            | NodePayload::InstantiationOutput { .. }
            | NodePayload::RegisterRead { .. } => {}
        }
    }
    live
}

fn main() {
    let sample_count = 512u64;
    let mut live_op_counts = BTreeMap::<String, usize>::new();
    let mut unsupported_counts = BTreeMap::<String, usize>::new();
    let mut converted = 0usize;

    for seed in 0..sample_count {
        let bytes = sample_bytes(seed, 256);
        let mut u = Unstructured::new(&bytes);
        let width = match u.int_in_range::<u8>(1..=8) {
            Ok(width) => usize::from(width),
            Err(_) => continue,
        };
        let (param_types, return_type) = recurrence_types(width);
        let sample = match generate_fuzz_sample_with_type(&mut u, &param_types, &return_type) {
            Ok(sample) => sample,
            Err(_) => continue,
        };
        let mut package = match IrPackage::new("fuzz_package") {
            Ok(package) => package,
            Err(_) => continue,
        };
        if generate_ir_fn(sample.ops, &mut package, Some(&return_type)).is_err() {
            continue;
        }

        let artifacts = match parse_ir_text(
            Path::new("fuzz_generated.ir"),
            &package.to_string(),
            Some("fuzz_test"),
        ) {
            Ok(artifacts) => artifacts,
            Err(_) => continue,
        };
        let top_fn = match artifacts.top_fn() {
            Ok(top_fn) => top_fn,
            Err(_) => continue,
        };
        for op in collect_live_ops(top_fn) {
            *live_op_counts.entry(op).or_default() += 1;
        }
        match export_pir_fn_to_lustre_step(top_fn) {
            Ok(_) => converted += 1,
            Err(err) => {
                let text = err.to_string();
                *unsupported_counts.entry(text).or_default() += 1;
            }
        }
    }

    println!("samples converted: {converted}/{sample_count}");
    println!("live ops:");
    for (op, count) in live_op_counts.iter().rev().take(live_op_counts.len()) {
        println!("  {op}: {count}");
    }
    println!("unsupported:");
    let mut unsupported: Vec<_> = unsupported_counts.into_iter().collect();
    unsupported.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    for (message, count) in unsupported.into_iter().take(20) {
        println!("  {count:>4}  {message}");
    }
}
