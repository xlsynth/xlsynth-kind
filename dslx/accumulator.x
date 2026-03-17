pub fn step(i: u32, s: u32) -> (u32, u32) {
    let o = i + s;
    let next_s = s + u32:1;
    (o, next_s)
}
