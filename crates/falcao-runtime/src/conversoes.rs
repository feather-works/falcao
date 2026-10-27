//! Conversões e comparações da linguagem (§7.1 e §7.2): ToBoolean, ToNumber, ToString,
//! ToPrimitive, ToObject, ToPropertyKey, typeof e as igualdades.

use crate::formas::{self, Classe};
use crate::funcoes::Res;
use crate::intrinsecos::simb;
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::numeros;
use crate::textos::ChaveProp;
use crate::valor::Valor;
use std::cmp::Ordering;

/// Dica de ToPrimitive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Dica {
    Padrao,
    Numero,
    Texto,
}

impl Motor {
    // ---------------------------------------------------------------- booleano

    pub fn verdadeiro(&mut self, v: Valor) -> bool {
        if v.e_int() {
            return v.0 != 0;
        }
        match v {
            Valor::TRUE => true,
            Valor::FALSE | Valor::UNDEFINED | Valor::NULL | Valor::BURACO | Valor::VAZIA => false,
            _ => match self.forma(v) {
                formas::NUMERO => {
                    let x = self.f64_de(v);
                    x != 0.0 && !x.is_nan()
                }
                formas::LATIN1 | formas::UTF16 | formas::CORDA => self.texto_len(v) > 0,
                formas::BIGINT => !self.bigint_e_zero(v),
                _ => true,
            },
        }
    }

    // ---------------------------------------------------------------- primitivo

    pub fn e_primitivo(&self, v: Valor) -> bool {
        !self.e_objeto(v)
    }

    /// ToPrimitive (§7.1.1).
    pub fn para_primitivo(&mut self, v: Valor, dica: Dica) -> Res<Valor> {
        if !self.e_objeto(v) {
            return Ok(v);
        }
        let exotico = self.obter(v, ChaveProp::Nome(simb::TO_PRIMITIVE))?;
        if !exotico.e_nulo_ou_indefinido() {
            if !self.e_funcao(exotico) {
                return self.erro_tipo("Symbol.toPrimitive não é uma função");
            }
            let d = match dica {
                Dica::Padrao => "default",
                Dica::Numero => "number",
                Dica::Texto => "string",
            };
            let d = self.texto(d);
            let r = self.chamar(exotico, v, &[d])?;
            if self.e_objeto(r) {
                return self.erro_tipo("Cannot convert object to primitive value");
            }
            return Ok(r);
        }
        let ordem = if dica == Dica::Texto { [fixos::TO_STRING, fixos::VALUE_OF] } else { [fixos::VALUE_OF, fixos::TO_STRING] };
        for nome in ordem {
            let f = self.obter(v, ChaveProp::Nome(nome))?;
            if self.e_funcao(f) {
                let r = self.chamar(f, v, &[])?;
                if !self.e_objeto(r) {
                    return Ok(r);
                }
            }
        }
        self.erro_tipo("Cannot convert object to primitive value")
    }

    // ---------------------------------------------------------------- números

    /// ToNumber (§7.1.4).
    pub fn para_numero(&mut self, v: Valor) -> Res<f64> {
        if v.e_int() {
            return Ok(v.como_int() as f64);
        }
        match v {
            Valor::UNDEFINED | Valor::BURACO => return Ok(f64::NAN),
            Valor::NULL | Valor::FALSE => return Ok(0.0),
            Valor::TRUE => return Ok(1.0),
            Valor::VAZIA => return Ok(0.0),
            _ => {}
        }
        match self.forma(v) {
            formas::NUMERO => Ok(self.f64_de(v)),
            formas::LATIN1 | formas::UTF16 | formas::CORDA => {
                let u = self.texto_u16_de(v);
                Ok(numeros::texto_para_numero(&u))
            }
            formas::SIMBOLO => self.erro_tipo("Cannot convert a Symbol value to a number"),
            formas::BIGINT => self.erro_tipo("Cannot convert a BigInt value to a number"),
            _ => {
                let p = self.para_primitivo(v, Dica::Numero)?;
                self.para_numero(p)
            }
        }
    }

    /// ToNumber devolvendo valor (sem caixa nova quando já é número).
    pub fn para_numero_valor(&mut self, v: Valor) -> Res<Valor> {
        if self.e_numero(v) {
            return Ok(v);
        }
        let x = self.para_numero(v)?;
        Ok(self.numero(x))
    }

    pub fn para_int32(&mut self, v: Valor) -> Res<i32> {
        if v.e_int() {
            return Ok(v.como_int());
        }
        let x = self.para_numero(v)?;
        Ok(numeros::para_int32(x))
    }

    pub fn para_uint32(&mut self, v: Valor) -> Res<u32> {
        if v.e_int() {
            return Ok(v.como_int() as u32);
        }
        let x = self.para_numero(v)?;
        Ok(numeros::para_uint32(x))
    }

    /// ToIntegerOrInfinity (§7.1.5).
    pub fn para_inteiro(&mut self, v: Valor) -> Res<f64> {
        if v.e_int() {
            return Ok(v.como_int() as f64);
        }
        let x = self.para_numero(v)?;
        Ok(if x.is_nan() { 0.0 } else { x.trunc() + 0.0 })
    }

    /// ToLength (§7.1.20).
    pub fn para_comprimento(&mut self, v: Valor) -> Res<f64> {
        let x = self.para_inteiro(v)?;
        Ok(x.clamp(0.0, 9_007_199_254_740_991.0))
    }

    // ---------------------------------------------------------------- textos

    pub fn numero_para_texto(&mut self, x: f64) -> Valor {
        if x == 0.0 {
            return self.caracteres[b'0' as usize];
        }
        let s = numeros::numero_para_texto(x);
        self.texto(&s)
    }

    /// ToString (§7.1.17), devolvendo o texto.
    pub fn para_texto(&mut self, v: Valor) -> Res<Valor> {
        if v.e_int() {
            let i = v.como_int();
            if (0..10).contains(&i) {
                return Ok(self.caracteres[b'0' as usize + i as usize]);
            }
            return Ok(self.texto(&i.to_string()));
        }
        Ok(match v {
            Valor::UNDEFINED | Valor::BURACO => self.texto_do_nome(fixos::UNDEFINED),
            Valor::NULL => self.texto_do_nome(fixos::NULL),
            Valor::TRUE => self.texto_do_nome(fixos::TRUE),
            Valor::FALSE => self.texto_do_nome(fixos::FALSE),
            _ => match self.forma(v) {
                formas::LATIN1 | formas::UTF16 | formas::CORDA => v,
                formas::NUMERO => {
                    let x = self.f64_de(v);
                    self.numero_para_texto(x)
                }
                formas::SIMBOLO => return self.erro_tipo("Cannot convert a Symbol value to a string"),
                formas::BIGINT => {
                    let s = self.ler_bigint(v).para_texto(10);
                    self.texto(&s)
                }
                _ => {
                    let p = self.para_primitivo(v, Dica::Texto)?;
                    return self.para_texto(p);
                }
            },
        })
    }

    /// ToString para uso em Rust.
    pub fn para_texto_rust(&mut self, v: Valor) -> Res<String> {
        let t = self.para_texto(v)?;
        Ok(self.texto_rust(t))
    }

    // ---------------------------------------------------------------- objetos e chaves

    /// ToObject (§7.1.18).
    pub fn para_objeto(&mut self, v: Valor) -> Res<Valor> {
        if self.e_objeto(v) {
            return Ok(v);
        }
        if v.e_nulo_ou_indefinido() || v == Valor::BURACO {
            return self.erro_tipo("Cannot convert undefined or null to object");
        }
        let proto = self.proto_do_primitivo(v);
        let f = self.forma_para(Classe::Caixa, proto, 0);
        let o = self.novo_objeto(f);
        self.por_interno_valor(o, 0, v);
        Ok(o)
    }

    /// Protótipo usado para ler propriedade de um primitivo.
    pub fn proto_do_primitivo(&self, v: Valor) -> Valor {
        if v.e_int() {
            return self.intr.numero_proto;
        }
        match v {
            Valor::TRUE | Valor::FALSE => self.intr.booleano_proto,
            _ => match self.forma(v) {
                formas::NUMERO => self.intr.numero_proto,
                formas::LATIN1 | formas::UTF16 | formas::CORDA => self.intr.texto_proto,
                formas::SIMBOLO => self.intr.simbolo_proto,
                formas::BIGINT => self.intr.bigint_proto,
                _ => self.intr.objeto_proto,
            },
        }
    }

    /// ToPropertyKey (§7.1.19).
    pub fn para_chave(&mut self, v: Valor) -> Res<ChaveProp> {
        if v.e_int() && v.como_int() >= 0 {
            return Ok(ChaveProp::Indice(v.como_int() as u32));
        }
        if v.e_ref() {
            match self.forma(v) {
                formas::LATIN1 | formas::UTF16 | formas::CORDA => return Ok(self.chave_de_texto(v)),
                formas::SIMBOLO => return Ok(ChaveProp::Nome(self.chave_do_simbolo(v))),
                formas::NUMERO => {
                    let x = self.f64_de(v);
                    if x >= 0.0 && x < 4_294_967_295.0 && x.fract() == 0.0 {
                        return Ok(ChaveProp::Indice(x as u32));
                    }
                }
                _ => {}
            }
        }
        let p = self.para_primitivo(v, Dica::Texto)?;
        if self.e_simbolo(p) {
            return Ok(ChaveProp::Nome(self.chave_do_simbolo(p)));
        }
        let t = self.para_texto(p)?;
        Ok(self.chave_de_texto(t))
    }

    /// A chave como valor JS (texto, ou o objeto símbolo).
    pub fn chave_para_valor(&mut self, k: ChaveProp) -> Valor {
        match k {
            ChaveProp::Indice(i) => {
                if i < 10 {
                    self.caracteres[b'0' as usize + i as usize]
                } else {
                    self.texto(&i.to_string())
                }
            }
            ChaveProp::Nome(c) if c.e_simbolo() => self.simbolos[(c.0 & !crate::formas::Chave::BIT_SIMBOLO) as usize],
            ChaveProp::Nome(c) => self.texto_do_nome(c),
        }
    }

    /// Texto da chave para mensagens.
    pub fn chave_para_rust(&mut self, k: ChaveProp) -> String {
        match k {
            ChaveProp::Indice(i) => i.to_string(),
            ChaveProp::Nome(c) if c.e_simbolo() => {
                let s = self.simbolos[(c.0 & !crate::formas::Chave::BIT_SIMBOLO) as usize];
                let d = self.descricao_do_simbolo(s);
                if d == Valor::UNDEFINED {
                    "Symbol()".into()
                } else {
                    format!("Symbol({})", self.texto_rust(d))
                }
            }
            ChaveProp::Nome(c) => self.nomes.texto_rust(c),
        }
    }

    // ---------------------------------------------------------------- typeof

    pub fn typeof_(&mut self, v: Valor) -> Valor {
        let nome = if self.e_numero(v) {
            fixos::NUMBER
        } else {
            match v {
                Valor::UNDEFINED | Valor::BURACO => fixos::UNDEFINED,
                Valor::NULL => fixos::OBJECT,
                Valor::TRUE | Valor::FALSE => fixos::BOOLEAN,
                _ => match self.forma(v) {
                    formas::LATIN1 | formas::UTF16 | formas::CORDA => fixos::STRING,
                    formas::SIMBOLO => fixos::SYMBOL,
                    formas::BIGINT => fixos::BIGINT,
                    _ if self.e_funcao(v) => fixos::FUNCTION,
                    _ => fixos::OBJECT,
                },
            }
        };
        self.texto_do_nome(nome)
    }

    // ---------------------------------------------------------------- igualdades

    /// `===` (§7.2.15).
    pub fn identicos(&mut self, a: Valor, b: Valor) -> bool {
        if a == b {
            // NaN em caixa: o mesmo objeto número que guarda NaN não é igual a si mesmo.
            return !(a.e_ref() && self.forma(a) == formas::NUMERO && self.f64_de(a).is_nan());
        }
        if a.e_int() && b.e_int() {
            return false;
        }
        if self.e_numero(a) && self.e_numero(b) {
            return self.f64_de(a) == self.f64_de(b);
        }
        if self.e_texto(a) && self.e_texto(b) {
            return self.textos_iguais(a, b);
        }
        if self.e_bigint(a) && self.e_bigint(b) {
            return self.ler_bigint(a) == self.ler_bigint(b);
        }
        false
    }

    /// SameValue (`Object.is`).
    pub fn mesmo_valor(&mut self, a: Valor, b: Valor) -> bool {
        if self.e_numero(a) && self.e_numero(b) {
            let (x, y) = (self.f64_de(a), self.f64_de(b));
            return (x.is_nan() && y.is_nan()) || (x == y && x.is_sign_negative() == y.is_sign_negative());
        }
        self.identicos(a, b)
    }

    /// SameValueZero (Map, Set, includes).
    pub fn mesmo_valor_zero(&mut self, a: Valor, b: Valor) -> bool {
        if self.e_numero(a) && self.e_numero(b) {
            let (x, y) = (self.f64_de(a), self.f64_de(b));
            return (x.is_nan() && y.is_nan()) || x == y;
        }
        self.identicos(a, b)
    }

    /// `==` (§7.2.14).
    pub fn iguais(&mut self, a: Valor, b: Valor) -> Res<bool> {
        let a = if a == Valor::BURACO { Valor::UNDEFINED } else { a };
        let b = if b == Valor::BURACO { Valor::UNDEFINED } else { b };
        if a == b || (self.e_numero(a) && self.e_numero(b)) || (self.e_texto(a) && self.e_texto(b)) || (self.e_bigint(a) && self.e_bigint(b)) {
            return Ok(self.identicos(a, b));
        }
        if a.e_nulo_ou_indefinido() || b.e_nulo_ou_indefinido() {
            return Ok(a.e_nulo_ou_indefinido() && b.e_nulo_ou_indefinido());
        }
        let (oa, ob) = (self.e_objeto(a), self.e_objeto(b));
        if oa && ob {
            return Ok(false);
        }
        if a == Valor::TRUE || a == Valor::FALSE {
            let x = Valor::int(i32::from(a == Valor::TRUE));
            return self.iguais(x, b);
        }
        if b == Valor::TRUE || b == Valor::FALSE {
            let y = Valor::int(i32::from(b == Valor::TRUE));
            return self.iguais(a, y);
        }
        if oa && !self.e_simbolo(b) {
            let p = self.para_primitivo(a, Dica::Padrao)?;
            return self.iguais(p, b);
        }
        if ob && !self.e_simbolo(a) {
            let p = self.para_primitivo(b, Dica::Padrao)?;
            return self.iguais(a, p);
        }
        if self.e_simbolo(a) || self.e_simbolo(b) {
            return Ok(false);
        }
        // BigInt com número ou texto (os dois BigInt já foram acima).
        if self.e_bigint(a) || self.e_bigint(b) {
            let (g, o) = if self.e_bigint(a) { (a, b) } else { (b, a) };
            let x = self.ler_bigint(g);
            if self.e_texto(o) {
                return Ok(self.texto_para_bigint(o).is_some_and(|y| y == x));
            }
            return Ok(self.e_numero(o) && x.cmp_f64(self.f64_de(o)) == Some(Ordering::Equal));
        }
        // Número com texto.
        let x = self.para_numero(a)?;
        let y = self.para_numero(b)?;
        Ok(x == y)
    }

    /// `a < b` (§7.2.13); `None` = indefinido (NaN).
    pub fn menor(&mut self, a: Valor, b: Valor, esquerda_primeiro: bool) -> Res<Option<bool>> {
        if a.e_int() && b.e_int() {
            return Ok(Some(a.como_int() < b.como_int()));
        }
        let (pa, pb) = if esquerda_primeiro {
            let pa = self.para_primitivo(a, Dica::Numero)?;
            (pa, self.para_primitivo(b, Dica::Numero)?)
        } else {
            let pb = self.para_primitivo(b, Dica::Numero)?;
            (self.para_primitivo(a, Dica::Numero)?, pb)
        };
        if self.e_texto(pa) && self.e_texto(pb) {
            return Ok(Some(self.comparar_textos(pa, pb) == Ordering::Less));
        }
        if self.e_bigint(pa) || self.e_bigint(pb) {
            return self.menor_bigint(pa, pb);
        }
        let x = self.para_numero(pa)?;
        let y = self.para_numero(pb)?;
        if x.is_nan() || y.is_nan() {
            return Ok(None);
        }
        Ok(Some(x < y))
    }

    /// `a < b` com algum BigInt (os dois já primitivos); texto vira BigInt, número compara exato.
    fn menor_bigint(&mut self, a: Valor, b: Valor) -> Res<Option<bool>> {
        let lado = |m: &mut Motor, v: Valor| -> Res<Option<Result<crate::grande::Grande, f64>>> {
            if m.e_bigint(v) {
                return Ok(Some(Ok(m.ler_bigint(v))));
            }
            if m.e_texto(v) {
                return Ok(m.texto_para_bigint(v).map(Ok));
            }
            Ok(Some(Err(m.para_numero(v)?)))
        };
        let (Some(x), Some(y)) = (lado(self, a)?, lado(self, b)?) else { return Ok(None) };
        let o = match (x, y) {
            (Ok(x), Ok(y)) => Some(x.cmp(&y)),
            (Ok(x), Err(y)) => x.cmp_f64(y),
            (Err(x), Ok(y)) => y.cmp_f64(x).map(Ordering::reverse),
            (Err(x), Err(y)) => x.partial_cmp(&y),
        };
        Ok(o.map(|o| o == Ordering::Less))
    }
}
