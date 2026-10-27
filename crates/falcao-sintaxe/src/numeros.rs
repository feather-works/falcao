//! Conversão dos literais numéricos para `f64`, com arredondamento correto (o mais próximo,
//! empate para o par), como manda o ECMA-262 §12.9.3.

/// Decimal já validado, sem `_` (ex.: `"12.5e-3"`, `".5"`, `"7."`).
pub(crate) fn decimal(texto: &str) -> f64 {
    texto.parse::<f64>().unwrap_or(f64::NAN)
}

/// Inteiro em `base` (2, 8, 10 ou 16) com os dígitos já validados, sem `_`.
pub(crate) fn inteiro(digitos: &str, base: u32) -> f64 {
    // Inteiro grande em "membros" de 32 bits, do menos significativo para o mais.
    let mut membros: Vec<u32> = vec![0];
    for c in digitos.chars() {
        let d = c.to_digit(base).unwrap_or(0);
        let mut vai = u64::from(d);
        for m in &mut membros {
            let v = u64::from(*m) * u64::from(base) + vai;
            *m = v as u32;
            vai = v >> 32;
        }
        if vai != 0 {
            membros.push(vai as u32);
        }
    }
    grande_para_f64(&membros)
}

fn grande_para_f64(membros: &[u32]) -> f64 {
    let Some(topo) = membros.iter().rposition(|&m| m != 0) else { return 0.0 };
    let bits = topo as u32 * 32 + (32 - membros[topo].leading_zeros());
    let bit = |i: u32| (membros[(i / 32) as usize] >> (i % 32)) & 1 == 1;
    if bits <= 64 {
        let mut v = 0u64;
        for i in (0..bits).rev() {
            v = (v << 1) | u64::from(bit(i));
        }
        // `as f64` arredonda para o mais próximo, empate para o par.
        return v as f64;
    }
    // Os 64 bits de cima; qualquer bit abaixo deles vira o bit 0 ("pegajoso"), que fica
    // abaixo do bit de arredondamento e só desempata o caso de metade exata.
    let corte = bits - 64;
    let mut v = 0u64;
    for i in (corte..bits).rev() {
        v = (v << 1) | u64::from(bit(i));
    }
    if (0..corte).any(bit) {
        v |= 1;
    }
    let mut r = v as f64;
    for _ in 0..corte {
        r *= 2.0;
        if r.is_infinite() {
            break;
        }
    }
    r
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn arredonda_inteiros_grandes_como_a_norma() {
        assert_eq!(inteiro("ff", 16), 255.0);
        assert_eq!(inteiro("777", 8), 511.0);
        // 2^53 + 1: metade exata, empate para o par (2^53).
        assert_eq!(inteiro("20000000000001", 16), 9007199254740992.0);
        // 2^53 + 3: arredonda para cima (2^53 + 4).
        assert_eq!(inteiro("20000000000003", 16), 9007199254740996.0);
        // Mais de 64 bits: 2^64 + 1 vira 2^64.
        assert_eq!(inteiro("10000000000000001", 16), 18446744073709551616.0);
        // Metade exata com bit pegajoso abaixo: 2^65 + 2^12 + 1 sobe.
        assert_eq!(inteiro("20000000000001001", 16), 36893488147419111424.0);
        assert!(inteiro(&"f".repeat(300), 16).is_infinite());
    }
}
