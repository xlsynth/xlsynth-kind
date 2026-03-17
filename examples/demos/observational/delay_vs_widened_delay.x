type FState = u8;
type GState = u16;

pub fn f(i: u8, s: FState) -> (u8, FState) {
    let o: u8 = s;
    let next_s: FState = i;
    (o, next_s)
}

pub fn g(i: u8, s: GState) -> (u8, GState) {
    let o: u8 = s as u8;
    let next_s: GState = i as u16;
    (o, next_s)
}
