//! Roda arquivos JS no Falcão: `cargo run --release --example rodar -- a.js [b.js…]`.

use falcao_runtime::Motor;
use std::time::Instant;

fn main() {
    let mut m = Motor::novo();
    for caminho in std::env::args().skip(1) {
        let fonte = std::fs::read_to_string(&caminho).expect("não li o arquivo");
        let t = Instant::now();
        match m.avaliar(&fonte) {
            Ok(_) => {}
            Err(e) => m.relatar_erro(e),
        }
        m.rodar_microtarefas();
        eprintln!("[{caminho}: {:.1} ms]", t.elapsed().as_secs_f64() * 1000.0);
    }
    if std::env::var_os("FALCAO_PERFIL").is_some() {
        eprint!("{}", m.relatorio_perfil());
    }
}
