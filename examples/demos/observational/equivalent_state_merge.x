type FState = u2;
type GState = u1;

pub fn f(i: u1, s: FState) -> (u1, FState) {
    let start = u2:0;
    let dup_a = u2:1;
    let dup_b = u2:2;
    let o: u1 = if s == start { u1:0 } else { u1:1 };
    let next_s: FState = if s == start {
        if i == u1:1 { dup_a } else { dup_b }
    } else if s == dup_a {
        if i == u1:1 { dup_a } else { start }
    } else if s == dup_b {
        if i == u1:1 { dup_a } else { start }
    } else {
        start
    };
    (o, next_s)
}

pub fn g(i: u1, s: GState) -> (u1, GState) {
    let start = u1:0;
    let merged = u1:1;
    let o: u1 = s;
    let next_s: GState = if s == start {
        merged
    } else if i == u1:1 {
        merged
    } else {
        start
    };
    (o, next_s)
}
