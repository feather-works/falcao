//! `BigInt` (§21.2) e a ponte entre o objeto BigInt da gaiola e a aritmética de
//! [`crate::grande`]. Layout: `[forma][len | sinal << 31][u32…]`, a palavra menos
//! significativa primeiro (`formas::Tipo::BigInt`). BigInt é imutável: as contas sempre criam
//! um novo.

use crate::conversoes::Dica;
use crate::formas::{self, attr, Classe};
use crate::funcoes::{Args, Res};
use crate::grande::Grande;
use crate::intrinsecos::{simb, TipoErro};
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::numeros;
use crate::valor::Valor;

const SINAL: u32 = 1 << 31;

pub(super) fn instalar(m: &mut Motor) {
    let g = m.intr.global;
    let bp = m.intr.bigint_proto;
    let ctor = m.nativa("BigInt", 1, bigint_construtor, false);
    m.definir_propria(ctor, fixos::PROTOTYPE, bp, 0);
    m.definir_propria(bp, fixos::CONSTRUCTOR, ctor, attr::METODO);
    m.metodo(ctor, "asIntN", 2, as_int_n);
    m.metodo(ctor, "asUintN", 2, as_uint_n);
    m.metodo(bp, "toString", 0, to_string);
    m.metodo(bp, "toLocaleString", 0, to_locale_string);
    m.metodo(bp, "valueOf", 0, value_of);
    let tag = m.texto("BigInt");
    m.definir_propria(bp, simb::TO_STRING_TAG, tag, attr::CONFIGURAVEL);
    let c = m.chave("BigInt");
    m.definir_propria(g, c, ctor, attr::METODO);
}

impl Motor {
    /// BigInt novo na gaiola.
    pub fn novo_bigint(&mut self, x: &Grande) -> Valor {
        self.gravar_bigint(x, false)
    }

    /// Grava o BigInt; `velho` para as constantes do bytecode (não mudam de lugar à toa).
    pub(crate) fn gravar_bigint(&mut self, x: &Grande, velho: bool) -> Valor {
        let n = x.mag.len() as u32;
        let d = if velho { self.g.alocar_velho(8 + 4 * n) } else { self.g.alocar(8 + 4 * n) };
        self.g.por_u32(d, formas::BIGINT.0);
        self.g.por_u32(d + 4, n | if x.neg { SINAL } else { 0 });
        for (i, &w) in x.mag.iter().enumerate() {
            self.g.por_u32(d + 8 + 4 * i as u32, w);
        }
        Valor::de_deslocamento(d)
    }

    /// O valor de um BigInt da gaiola.
    pub fn ler_bigint(&self, v: Valor) -> Grande {
        let d = v.deslocamento();
        let w = self.g.u32_em(d + 4);
        let mag = (0..w & !SINAL).map(|i| self.g.u32_em(d + 8 + 4 * i)).collect();
        Grande { neg: w & SINAL != 0, mag }
    }

    /// `0n` é o único BigInt falso.
    pub(crate) fn bigint_e_zero(&self, v: Valor) -> bool {
        self.g.u32_em(v.deslocamento() + 4) & !SINAL == 0
    }

    /// Resultado de conta BigInt: o `Err` vira RangeError.
    pub(crate) fn bigint_ou_alcance(&mut self, r: Result<Grande, &'static str>) -> Res<Valor> {
        match r {
            Ok(x) => Ok(self.novo_bigint(&x)),
            Err(msg) => self.erro_alcance(msg),
        }
    }

    pub(crate) fn erro_mistura<T>(&mut self) -> Res<T> {
        self.erro_tipo("Cannot mix BigInt and other types, use explicit conversions")
    }

    /// StringToBigInt (§7.1.14): `None` se o texto não é um inteiro válido.
    pub fn texto_para_bigint(&mut self, t: Valor) -> Option<Grande> {
        let u = self.texto_u16_de(t);
        let ini = u.iter().position(|&c| !numeros::e_espaco(c)).unwrap_or(u.len());
        let fim = u.iter().rposition(|&c| !numeros::e_espaco(c)).map_or(ini, |i| i + 1);
        let s: Vec<u8> = u[ini..fim].iter().map(|&c| if c < 128 { c as u8 } else { b'!' }).collect();
        let (base, digitos, neg) = match s.as_slice() {
            [] => return Some(Grande::zero()),
            [b'0', b'x' | b'X', r @ ..] => (16, r, false),
            [b'0', b'o' | b'O', r @ ..] => (8, r, false),
            [b'0', b'b' | b'B', r @ ..] => (2, r, false),
            [b'-', r @ ..] => (10, r, true),
            [b'+', r @ ..] => (10, r, false),
            r => (10, r, false),
        };
        if digitos.is_empty() {
            return None;
        }
        let x = Grande::de_digitos(digitos, base)?;
        Some(if neg { x.negar() } else { x })
    }

    /// ToBigInt (§7.1.13).
    pub fn para_bigint(&mut self, v: Valor) -> Res<Grande> {
        let p = self.para_primitivo(v, Dica::Numero)?;
        match p {
            Valor::TRUE => return Ok(Grande::de_i64(1)),
            Valor::FALSE => return Ok(Grande::zero()),
            Valor::UNDEFINED | Valor::NULL | Valor::BURACO => {
                let s = self.para_texto_rust(p)?;
                return self.erro_tipo(&format!("Cannot convert {s} to a BigInt"));
            }
            _ => {}
        }
        if self.e_bigint(p) {
            return Ok(self.ler_bigint(p));
        }
        if self.e_texto(p) {
            return match self.texto_para_bigint(p) {
                Some(x) => Ok(x),
                None => {
                    let s = self.texto_rust(p);
                    Err(self.erro(TipoErro::Sintaxe, &format!("Cannot convert {s} to a BigInt")))
                }
            };
        }
        if self.e_numero(p) {
            let s = self.para_texto_rust(p)?;
            return self.erro_tipo(&format!("Cannot convert {s} to a BigInt"));
        }
        self.erro_tipo("Cannot convert a Symbol value to a BigInt")
    }

    /// ToNumeric (§7.1.3): número, ou o próprio BigInt.
    pub fn para_numerico(&mut self, v: Valor) -> Res<Valor> {
        if self.e_numero(v) || self.e_bigint(v) {
            return Ok(v);
        }
        let p = self.para_primitivo(v, Dica::Numero)?;
        if self.e_bigint(p) {
            return Ok(p);
        }
        self.para_numero_valor(p)
    }
}

fn bigint_construtor(m: &mut Motor, _this: Valor, a: Args, nt: Valor) -> Res<Valor> {
    if nt != Valor::UNDEFINED {
        return m.erro_tipo("BigInt is not a constructor");
    }
    let v = m.arg(a, 0);
    let p = m.para_primitivo(v, Dica::Numero)?;
    if m.e_numero(p) {
        let x = m.f64_de(p);
        return match Grande::de_f64(x) {
            Some(g) => Ok(m.novo_bigint(&g)),
            None => {
                let s = numeros::numero_para_texto(x);
                m.erro_alcance(&format!("The number {s} cannot be converted to a BigInt because it is not an integer"))
            }
        };
    }
    let g = m.para_bigint(p)?;
    Ok(m.novo_bigint(&g))
}

/// ToIndex (§7.1.22) dos `bits`.
fn bits_de(m: &mut Motor, v: Valor) -> Res<u64> {
    let x = m.para_inteiro(v)?;
    if !(0.0..=9_007_199_254_740_991.0).contains(&x) {
        return m.erro_alcance("Invalid value: not (convertible to) a safe integer");
    }
    Ok(x as u64)
}

fn como_n(m: &mut Motor, a: Args, com_sinal: bool) -> Res<Valor> {
    let (b, v) = (m.arg(a, 0), m.arg(a, 1));
    let bits = bits_de(m, b)?;
    let x = m.para_bigint(v)?;
    let r = x.como_n_bits(bits, com_sinal);
    m.bigint_ou_alcance(r)
}

fn as_int_n(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    como_n(m, a, true)
}

fn as_uint_n(m: &mut Motor, _this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    como_n(m, a, false)
}

/// thisBigIntValue: BigInt ou caixa de BigInt.
fn bigint_this(m: &mut Motor, this: Valor, nome: &str) -> Res<Valor> {
    if m.e_bigint(this) {
        return Ok(this);
    }
    if m.e_objeto(this) && m.formas.get(m.forma(this)).classe == Classe::Caixa {
        let p = Valor(m.interno(this, 0));
        if m.e_bigint(p) {
            return Ok(p);
        }
    }
    m.erro_tipo(&format!("BigInt.prototype.{nome} requires that 'this' be a BigInt"))
}

fn to_string(m: &mut Motor, this: Valor, a: Args, _nt: Valor) -> Res<Valor> {
    let v = bigint_this(m, this, "toString")?;
    let rv = m.arg(a, 0);
    let raiz = if rv == Valor::UNDEFINED { 10.0 } else { m.para_inteiro(rv)? };
    if !(2.0..=36.0).contains(&raiz) {
        return m.erro_alcance("toString() radix must be between 2 and 36");
    }
    let s = m.ler_bigint(v).para_texto(raiz as u32);
    Ok(m.texto(&s))
}

// shortcut: toLocaleString sem separador de milhar nem opções; upgrade quando houver Intl.
fn to_locale_string(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let v = bigint_this(m, this, "toLocaleString")?;
    let s = m.ler_bigint(v).para_texto(10);
    Ok(m.texto(&s))
}

fn value_of(m: &mut Motor, this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    bigint_this(m, this, "valueOf")
}

#[cfg(test)]
mod testes {
    use crate::Motor;

    /// Roda o script e devolve o resultado como texto.
    fn rodar(fonte: &str) -> String {
        let mut m = Motor::novo();
        let v = m.avaliar(fonte).unwrap_or_else(|e| panic!("{}", m.texto_do_erro(e)));
        m.para_texto_rust(v).unwrap()
    }

    /// Cada expressão roda num `try`; o erro vira `NomeDoErro`.
    fn cada(exprs: &[&str]) -> String {
        let corpo: Vec<String> = exprs.iter().map(|e| format!("(() => {{ try {{ return String({e}); }} catch (e) {{ return e.name; }} }})()")).collect();
        rodar(&format!("[{}].join('|')", corpo.join(",")))
    }

    #[test]
    fn literais_e_typeof() {
        assert_eq!(
            cada(&["123n", "0x1Fn", "0o7n", "0b101n", "0n", "typeof 1n", "123456789012345678901234567890n", "-0n", "0xffffffffffffffffffffn"]),
            "123|31|7|5|0|bigint|123456789012345678901234567890|0|1208925819614629174706175"
        );
    }

    #[test]
    fn construtor_e_metodos() {
        assert_eq!(
            cada(&[
                "BigInt(42)", "BigInt(-0)", "BigInt(1e21)", "BigInt(1.5)", "BigInt(NaN)", "BigInt(' 0x10 ')", "BigInt('-12')",
                "BigInt('')", "BigInt('1.5')", "BigInt('-0x1')", "BigInt('12n')", "BigInt(true)", "BigInt(false)",
                "BigInt(undefined)", "BigInt(null)", "BigInt(Symbol())", "new BigInt(1)", "BigInt(10n)",
            ]),
            "42|0|1000000000000000000000|RangeError|RangeError|16|-12|0|SyntaxError|SyntaxError|SyntaxError|1|0|TypeError|TypeError|TypeError|TypeError|10"
        );
        assert_eq!(
            cada(&[
                "BigInt.asIntN(8, 255n)", "BigInt.asIntN(8, 127n)", "BigInt.asUintN(8, -1n)", "BigInt.asUintN(64, -1n)",
                "BigInt.asIntN(64, 2n ** 63n)", "BigInt.asIntN(-1, 1n)", "BigInt.asUintN(8, 1)",
                "(255n).toString(16)", "(-255n).toString(2)", "(10n).toString(37)", "(10n).valueOf() === 10n",
                "(1234n).toLocaleString()", "Object(5n).valueOf()", "typeof Object(5n)", "BigInt.prototype.toString.call(1)",
                "Object.prototype.toString.call(1n)",
            ]),
            "-1|127|255|18446744073709551615|-9223372036854775808|RangeError|TypeError|ff|-11111111|RangeError|true|1234|5|object|TypeError|[object BigInt]"
        );
    }

    #[test]
    fn aritmetica() {
        assert_eq!(
            cada(&[
                "2n ** 64n", "(2n ** 64n) - 1n", "7n / 2n", "-7n / 2n", "7n % -2n", "-7n % 2n", "1n / 0n", "1n % 0n", "2n ** -1n",
                "123456789012345678901234567890n * 987654321098765432109876543210n",
                "(2n ** 200n + 12345n) / (2n ** 70n + 3n)", "(2n ** 200n + 12345n) % (2n ** 70n + 3n)",
                "1n + 1", "1n * 2", "1 - 1n", "+1n", "-(5n)", "1n + 1n", "(-2n) ** 3n", "0n ** 0n",
            ]),
            "18446744073709551616|18446744073709551615|3|-3|1|-1|RangeError|RangeError|RangeError|\
121932631137021795226185032733622923332237463801111263526900|\
1361129467683753853850039665213252304896|10376293541461635129|\
TypeError|TypeError|TypeError|TypeError|-5|2|-8|1"
        );
    }

    #[test]
    fn bits_e_deslocamentos() {
        assert_eq!(
            cada(&[
                "1n << 64n", "1n << BigInt(3)", "-5n >> 1n", "-1n >> 100n", "256n >> 4n", "8n << -2n", "-5n & 3n", "-5n | 3n", "-5n ^ 3n",
                "~0n", "~-1n", "5n >>> 1n", "1n << 1", "(1n << 100n) | 1n",
            ]),
            "18446744073709551616|8|-3|-1|16|2|3|-5|-8|-1|0|TypeError|TypeError|1267650600228229401496703205377"
        );
    }

    #[test]
    fn incremento_e_atribuicao() {
        assert_eq!(rodar("let a = 9n; a++; ++a; let b = a--; let c = 0n; c += 5n; c *= 3n; c -= 1n; [a, b, c, typeof a].join()"), "10,11,14,bigint");
    }

    #[test]
    fn comparacoes_e_igualdades() {
        assert_eq!(
            cada(&[
                "1n < 2", "2n > 1.5", "1n < 1.5", "-1n < -0.5", "2n <= 2", "1n < '2'", "'10' > 9n", "1n < 'x'", "1n >= 'x'", "1n < NaN",
                "(2n ** 64n) > 1e19", "(2n ** 64n) < Infinity", "1n == 1", "1n == '1'", "1n == true", "0n == ''", "1n == 'x'",
                "1n === 1", "10n === 10n", "Object.is(5n, 5n)", "[1n, 2n].includes(2n)", "[1n].indexOf(1n)", "1n != 2n",
                "new Set([1n, 1n, 2n]).size", "new Map([[3n, 'a']]).get(3n)", "new Set([1n, 1]).size",
            ]),
            "true|true|true|true|true|true|true|false|false|false|true|true|true|true|true|true|false|false|true|true|true|0|true|2|a|2"
        );
    }

    #[test]
    fn conversoes() {
        assert_eq!(
            cada(&[
                "String(10n)", "`${-3n}`", "1n + ''", "'x' + 2n", "Number(10n)", "Number(2n ** 64n)", "!!0n", "!!1n", "0n ? 'v' : 'f'",
                "JSON.stringify(1n)", "JSON.stringify({a: 1n})", "Math.max(1n)", "parseInt(15n)", "({[2n]: 'ok'})['2']", "isNaN(1n)",
            ]),
            "10|-3|1|x2|10|18446744073709552000|false|true|f|TypeError|TypeError|TypeError|15|ok|TypeError"
        );
    }

    /// O que o Tailwind faz com BigInt para ordenar as camadas.
    #[test]
    fn ordenacao_como_no_tailwind() {
        assert_eq!(
            rodar(
                "const sinal = v => (v > 0n) - (v < 0n);
                 let base = 0n, camadas = {};
                 for (const [i, n] of ['base', 'components', 'utilities'].entries()) camadas[n] = 1n << BigInt(i);
                 const xs = [camadas.utilities | 1n << 10n, camadas.base, camadas.components | 1n << 3n, camadas.utilities];
                 xs.sort((a, b) => sinal(a - b));
                 xs.map(String).join() + '|' + (xs[0] & camadas.base ? 'b' : '-') + '|' + sinal(-5n)"
            ),
            "1,4,10,1028|b|-1"
        );
    }
}
