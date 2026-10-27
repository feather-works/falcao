//! Operadores binários e unários (§13), com atalho para inteiros pequenos.

use crate::conversoes::Dica;
use crate::formas::Classe;
use crate::funcoes::Res;
use crate::grande::{Grande, MAX_BITS};
use crate::intrinsecos::simb;
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::numeros;
use crate::textos::ChaveProp;
use crate::valor::Valor;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Arit {
    Sub,
    Mul,
    Div,
    Resto,
    Potencia,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Bits {
    E,
    Ou,
    Xor,
    Esq,
    Dir,
    DirSemSinal,
}

/// Potência do JS (`Math.pow` e `**`): difere do `powf` em NaN e em ±1 elevado a ±∞.
pub fn potencia(b: f64, e: f64) -> f64 {
    if e.is_nan() || (b.abs() == 1.0 && e.is_infinite()) {
        return f64::NAN;
    }
    b.powf(e)
}

impl Motor {
    /// `a + b`.
    pub fn somar(&mut self, a: Valor, b: Valor) -> Res<Valor> {
        if a.e_int() && b.e_int() {
            let s = i64::from(a.como_int()) + i64::from(b.como_int());
            return Ok(Valor::int_se_cabe(s).unwrap_or_else(|| self.caixa_numero(s as f64)));
        }
        if self.e_numero(a) && self.e_numero(b) {
            let x = self.f64_de(a) + self.f64_de(b);
            return Ok(self.numero(x));
        }
        if self.e_texto(a) && self.e_texto(b) {
            return self.concatenar(a, b);
        }
        let pa = self.para_primitivo(a, Dica::Padrao)?;
        let pb = self.para_primitivo(b, Dica::Padrao)?;
        if self.e_texto(pa) || self.e_texto(pb) {
            let ta = self.para_texto(pa)?;
            let tb = self.para_texto(pb)?;
            return self.concatenar(ta, tb);
        }
        let pa = self.para_numerico(pa)?;
        let pb = self.para_numerico(pb)?;
        if let Some((x, y)) = self.dois_bigint(pa, pb)? {
            return Ok(self.novo_bigint(&x.somar(&y)));
        }
        let (x, y) = (self.f64_de(pa), self.f64_de(pb));
        Ok(self.numero(x + y))
    }

    pub fn aritmetica(&mut self, op: Arit, a: Valor, b: Valor) -> Res<Valor> {
        if a.e_int() && b.e_int() {
            let (x, y) = (i64::from(a.como_int()), i64::from(b.como_int()));
            match op {
                Arit::Sub => return Ok(Valor::int_se_cabe(x - y).unwrap_or_else(|| self.caixa_numero((x - y) as f64))),
                Arit::Mul => {
                    let p = x * y;
                    if p != 0 || (x >= 0 && y >= 0) {
                        return Ok(Valor::int_se_cabe(p).unwrap_or_else(|| self.caixa_numero(p as f64)));
                    }
                }
                Arit::Resto if y != 0 => {
                    let r = x % y;
                    if r != 0 || x >= 0 {
                        return Ok(Valor::int(r as i32));
                    }
                }
                _ => {}
            }
        }
        let a = self.para_numerico(a)?;
        let b = self.para_numerico(b)?;
        if let Some((x, y)) = self.dois_bigint(a, b)? {
            let r = match op {
                Arit::Sub => Ok(x.subtrair(&y)),
                Arit::Mul if x.bits() + y.bits() > MAX_BITS + 1 => Err("Maximum BigInt size exceeded"),
                Arit::Mul => Ok(x.multiplicar(&y)),
                Arit::Div | Arit::Resto => match x.dividir(&y) {
                    Some((q, r)) => Ok(if op == Arit::Div { q } else { r }),
                    None => Err("Division by zero"),
                },
                Arit::Potencia => x.potencia(&y),
            };
            return self.bigint_ou_alcance(r);
        }
        let (x, y) = (self.f64_de(a), self.f64_de(b));
        let r = match op {
            Arit::Sub => x - y,
            Arit::Mul => x * y,
            Arit::Div => x / y,
            Arit::Resto => x % y,
            Arit::Potencia => potencia(x, y),
        };
        Ok(self.numero(r))
    }

    pub fn bits(&mut self, op: Bits, a: Valor, b: Valor) -> Res<Valor> {
        // Número com número (o `x | 0` de sempre) não precisa olhar BigInt.
        let numeros = self.e_numero(a) && self.e_numero(b);
        let (a, b) = if numeros { (a, b) } else { (self.para_numerico(a)?, self.para_numerico(b)?) };
        if let Some((x, y)) = if numeros { None } else { self.dois_bigint(a, b)? } {
            let r = match op {
                Bits::E => Ok(x.bit_a_bit(&y, |p, q| p & q)),
                Bits::Ou => Ok(x.bit_a_bit(&y, |p, q| p | q)),
                Bits::Xor => Ok(x.bit_a_bit(&y, |p, q| p ^ q)),
                Bits::Esq => x.deslocar(&y),
                Bits::Dir => x.deslocar(&y.negar()),
                Bits::DirSemSinal => return self.erro_tipo("BigInts have no unsigned right shift, use >> instead"),
            };
            return self.bigint_ou_alcance(r);
        }
        let x = self.para_int32(a)?;
        let y = self.para_int32(b)?;
        let r = match op {
            Bits::E => x & y,
            Bits::Ou => x | y,
            Bits::Xor => x ^ y,
            Bits::Esq => x.wrapping_shl(y as u32 & 31),
            Bits::Dir => x >> (y as u32 & 31),
            Bits::DirSemSinal => {
                let u = (x as u32) >> (y as u32 & 31);
                return Ok(self.numero(f64::from(u)));
            }
        };
        Ok(Valor::int_se_cabe(i64::from(r)).unwrap_or_else(|| self.caixa_numero(f64::from(r))))
    }

    /// `x + i` com `i` pequeno (incremento, `SomaInt`).
    pub fn somar_int(&mut self, a: Valor, i: i32) -> Res<Valor> {
        if a.e_int() {
            let s = i64::from(a.como_int()) + i64::from(i);
            return Ok(Valor::int_se_cabe(s).unwrap_or_else(|| self.caixa_numero(s as f64)));
        }
        if self.e_numero(a) {
            let x = self.f64_de(a) + f64::from(i);
            return Ok(self.numero(x));
        }
        // O gerador só emite imediato positivo para `+`; negativo vem de `x - n`, que converte
        // para número (`"5" - 1` é 4, não "5-1").
        if i < 0 {
            return self.aritmetica(Arit::Sub, a, Valor::int(-i));
        }
        self.somar(a, Valor::int(i))
    }

    pub fn negativo(&mut self, a: Valor) -> Res<Valor> {
        if a.e_int() && a.como_int() != 0 {
            return Ok(Valor::int(-a.como_int()));
        }
        let a = self.para_numerico(a)?;
        if self.e_bigint(a) {
            let x = self.ler_bigint(a).negar();
            return Ok(self.novo_bigint(&x));
        }
        let x = self.f64_de(a);
        Ok(self.numero(-x))
    }

    pub fn til(&mut self, a: Valor) -> Res<Valor> {
        let a = self.para_numerico(a)?;
        if self.e_bigint(a) {
            let x = self.ler_bigint(a).nao();
            return Ok(self.novo_bigint(&x));
        }
        let x = self.para_int32(a)?;
        Ok(Valor::int_se_cabe(i64::from(!x)).unwrap_or_else(|| self.caixa_numero(f64::from(!x))))
    }

    /// `a instanceof f` (§13.10.2).
    pub fn instancia_de(&mut self, a: Valor, f: Valor) -> Res<bool> {
        if !self.e_objeto(f) {
            return self.erro_tipo("Right-hand side of 'instanceof' is not an object");
        }
        let h = self.obter(f, ChaveProp::Nome(simb::HAS_INSTANCE))?;
        if !h.e_nulo_ou_indefinido() && h != self.intr.funcao_has_instance {
            let r = self.chamar(h, f, &[a])?;
            return Ok(self.verdadeiro(r));
        }
        if !self.e_funcao(f) {
            return self.erro_tipo("Right-hand side of 'instanceof' is not callable");
        }
        self.instancia_comum(f, a)
    }

    /// OrdinaryHasInstance.
    pub fn instancia_comum(&mut self, mut f: Valor, a: Valor) -> Res<bool> {
        if !self.e_funcao(f) {
            return Ok(false);
        }
        while self.formas.get(self.forma(f)).classe == Classe::FuncaoLigada {
            f = Valor(self.interno(f, 0));
        }
        if !self.e_objeto(a) {
            return Ok(false);
        }
        let p = self.obter(f, ChaveProp::Nome(fixos::PROTOTYPE))?;
        if !self.e_objeto(p) {
            return self.erro_tipo("Function has non-object prototype in instanceof check");
        }
        let mut o = self.proto_de(a);
        while o != Valor::NULL {
            if o == p {
                return Ok(true);
            }
            o = self.proto_de(o);
        }
        Ok(false)
    }

    /// Número para texto numa base (para `toString(raiz)`).
    pub fn numero_texto_raiz(&mut self, x: f64, raiz: u32) -> Valor {
        let s = if raiz == 10 { numeros::numero_para_texto(x) } else { numeros::numero_para_texto_raiz(x, raiz) };
        self.texto(&s)
    }

    /// Operandos já em ToNumeric: os dois BigInt dão `Some`; só um deles é TypeError.
    fn dois_bigint(&mut self, a: Valor, b: Valor) -> Res<Option<(Grande, Grande)>> {
        match (self.e_bigint(a), self.e_bigint(b)) {
            (true, true) => Ok(Some((self.ler_bigint(a), self.ler_bigint(b)))),
            (false, false) => Ok(None),
            _ => self.erro_mistura(),
        }
    }
}
