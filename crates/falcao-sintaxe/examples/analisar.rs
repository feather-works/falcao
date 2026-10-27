//! Lê arquivos JS inteiros com a sintaxe e mede o tempo (pastas: todos os `.js` dentro):
//!
//!     cargo run --release -p falcao-sintaxe --example analisar -- arquivo.js pasta/ ...

use falcao_sintaxe::{analisar_escopos, analisar_script};
use std::time::Instant;

fn main() {
    let mut arquivos = Vec::new();
    for a in std::env::args().skip(1) {
        let c = std::path::PathBuf::from(&a);
        if c.is_dir() {
            let mut d: Vec<_> = std::fs::read_dir(&c).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "js")).collect();
            d.sort();
            arquivos.extend(d);
        } else {
            arquivos.push(c);
        }
    }
    let (mut ok, mut bytes, mut total) = (0, 0, 0.0);
    for c in &arquivos {
        let Ok(fonte) = std::fs::read_to_string(c) else { continue };
        let mut melhor = f64::MAX;
        let mut erro = None;
        for _ in 0..3 {
            let t = Instant::now();
            let r = analisar_script(&fonte).map(|(a, l)| (analisar_escopos(&a), a, l));
            melhor = melhor.min(t.elapsed().as_secs_f64() * 1000.0);
            if let Err(e) = r {
                erro = Some(e);
                break;
            }
        }
        match erro {
            None => {
                ok += 1;
                bytes += fonte.len();
                total += melhor;
                if fonte.len() > 300_000 {
                    println!("{:>8.2} ms  {:>8} B  {}", melhor, fonte.len(), c.display());
                }
            }
            Some(e) => {
                let i = e.posicao as usize;
                let ini = fonte.floor_char_boundary(i.saturating_sub(40));
                let fim = fonte.ceil_char_boundary((i + 40).min(fonte.len()));
                println!("ERRO {}: {} em {}: …{}…", c.display(), e.mensagem, i, &fonte[ini..fim].replace('\n', "⏎"));
            }
        }
    }
    println!("{ok}/{} sem erro; {:.1} MB em {:.1} ms", arquivos.len(), bytes as f64 / 1e6, total);
}
