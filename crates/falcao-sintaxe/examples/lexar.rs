//! Lê um arquivo JS inteiro só com o léxico e mede o tempo:
//!
//!     cargo run --release -p falcao-sintaxe --example lexar -- arquivo.js
//!
//! O `/` é decidido pelo símbolo anterior (a sintaxe fará isso direito; aqui é só medida).

use falcao_sintaxe::atomos::reservada;
use falcao_sintaxe::{palavras, Dado, Lexico, Objetivo, Pont, Tipo};
use std::time::Instant;

fn main() {
    let caminho = std::env::args().nth(1).expect("uso: lexar arquivo.js");
    let fonte = std::fs::read_to_string(&caminho).expect("arquivo UTF-8");
    for rodada in 0..3 {
        let t = Instant::now();
        let mut l = Lexico::novo(&fonte, false);
        let mut n = 0usize;
        let mut anterior = Tipo::Fim;
        let mut palavra_antes = false;
        // Uma entrada por `${` aberto: quantas `{` comuns estão abertas dentro dele.
        let mut templates: Vec<u32> = Vec::new();
        loop {
            let obj = match anterior {
                Tipo::Nome if !palavra_antes => Objetivo::Divisao,
                Tipo::Numero | Tipo::Texto | Tipo::Regex | Tipo::BigInt | Tipo::NomePrivado => Objetivo::Divisao,
                Tipo::Pont(Pont::FechaParen | Pont::FechaColchete | Pont::FechaChave) => Objetivo::Divisao,
                _ => Objetivo::Regex,
            };
            let s = match l.proximo(obj).and_then(|s| {
                match s.tipo {
                    Tipo::Pont(Pont::AbreChave) => {
                        if let Some(t) = templates.last_mut() {
                            *t += 1;
                        }
                    }
                    Tipo::Pont(Pont::FechaChave) if templates.last() == Some(&0) => {
                        templates.pop();
                        return l.continuar_template(&s);
                    }
                    Tipo::Pont(Pont::FechaChave) => {
                        if let Some(t) = templates.last_mut() {
                            *t -= 1;
                        }
                    }
                    _ => {}
                }
                Ok(s)
            }) {
                Ok(s) => s,
                Err(e) => panic!("erro em {}: {} ({:?})", e.posicao, e.mensagem, &fonte[e.posicao as usize..(e.posicao as usize + 40).min(fonte.len())]),
            };
            if s.tipo == Tipo::Fim {
                break;
            }
            if let Tipo::Template(t) = s.tipo {
                if !t.cauda {
                    templates.push(0);
                }
            }
            palavra_antes = matches!(s.dado, Dado::Atomo(a) if s.tipo == Tipo::Nome && reservada(a) && ![palavras::THIS, palavras::SUPER, palavras::NULL, palavras::TRUE, palavras::FALSE].contains(&a));
            anterior = s.tipo;
            n += 1;
        }
        let dt = t.elapsed();
        if rodada == 2 {
            println!("{} kB, {n} símbolos, {} átomos: {:.1} ms ({:.0} MB/s)", fonte.len() / 1024, l.atomos.len(), dt.as_secs_f64() * 1e3, fonte.len() as f64 / dt.as_secs_f64() / 1e6);
        }
    }
}
