type FState = u8;
type GState = u16;

pub fn f(i: u8, s: FState) -> (u8, FState) {
    let o: u8 = i << u8:1;
    let next_s: FState = s + u8:1;
    (o, next_s)
}

pub fn g(i: u8, s: GState) -> (u8, GState) {
    let i_wide: GState = i as u16;
    let shifted: GState = i_wide << u16:1;
    let o: u8 = shifted as u8;
    let next_s: GState = s + u16:1;
    (o, next_s)
}
