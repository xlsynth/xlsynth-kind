type FState = u1;
type GState = u8;

pub fn f(i: u8, s: FState) -> (u1, FState) {
    let _ = i;
    let o: u1 = s;
    let next_s: FState = !s;
    (o, next_s)
}

pub fn g(i: u8, s: GState) -> (u1, GState) {
    let _ = i;
    let o: u1 = s as u1;
    let next_s: GState = s + u8:1;
    (o, next_s)
}
