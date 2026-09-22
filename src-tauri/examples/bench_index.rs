fn main() {
    let t = std::time::Instant::now();
    let hit = cxmail_lib::email::autoconfig::lookup_bundled("posteo.de");
    println!("first lookup (builds the whole 969-domain index): {:?}  hit={}", t.elapsed(), hit.is_some());
    let t = std::time::Instant::now();
    let _ = cxmail_lib::email::autoconfig::lookup_bundled("gmx.net");
    println!("second lookup (cached):                           {:?}", t.elapsed());
}
