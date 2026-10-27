//! Funções: as de bytecode (protótipo carregado + contexto) e as nativas (Rust).
//!
//! O objeto função tem 4 campos internos: `[código][contexto][casa][this da seta]`. O código é
//! o índice em [`Motor::protos`] ou, com o bit 31, em [`Motor::nativas`].

use crate::formas::{attr, Chave, Classe, FormaId, TipoElementos};
use crate::motor::Motor;
use crate::nomes::fixos;
use crate::valor::Valor;
use falcao_bytecode::{Constante, Script};
use std::rc::Rc;

/// Resultado de operação que pode lançar exceção (o `Err` é o valor lançado).
pub type Res<T> = Result<T, Valor>;

/// Argumentos de uma chamada: uma faixa da pilha do motor.
#[derive(Clone, Copy, Debug)]
pub struct Args {
    pub base: u32,
    pub n: u32,
}

/// Função nativa: `(motor, this, argumentos, new.target ou undefined)`.
pub type FnNativa = fn(&mut Motor, Valor, Args, Valor) -> Res<Valor>;

pub struct Nativa {
    pub f: FnNativa,
    pub nome: &'static str,
    pub construtor: bool,
}

pub const BIT_NATIVA: u32 = 1 << 31;

// Campos internos do objeto função.
pub const F_CODIGO: u32 = 0;
pub const F_CONTEXTO: u32 = 1;
pub const F_CASA: u32 = 2;
pub const F_THIS: u32 = 3;

/// Constante já traduzida para este motor.
#[derive(Clone, Copy, Debug)]
pub enum ConstRt {
    Valor(Valor),
    Nome(Chave),
    /// Função interna: índice em `Motor::protos`.
    Funcao(u32),
    /// Classe: protótipo do construtor (u32::MAX = padrão), derivada, nome.
    Classe { construtor: u32, derivada: bool, nome: Valor },
    /// Template marcado (o array é criado na primeira vez e guardado aqui).
    Template(u32),
    Regex,
}

/// Cache inline de propriedade (`ARQUITETURA.md`, seção 8). A entrada daqui é a primeira forma
/// vista (monomórfico); as seguintes vão para `Motor::poli` (até 4 formas no total) e, passando
/// disso, o site fica megamórfico e usa a tabela global `Motor::mega` (forma × nome).
#[derive(Clone, Copy, Debug)]
pub struct CacheProp {
    /// Forma do receptor (u32::MAX = vazio).
    pub forma: u32,
    /// Slot do campo (ver `Motor::ler_campo`).
    pub deslocamento: u32,
    /// Achado no protótipo: o objeto que tem a propriedade e a forma dele.
    pub dono: Valor,
    pub forma_dono: u32,
    /// Índice em `Motor::poli`, [`SEM_POLI`] ou [`MEGA`].
    pub poli: u32,
}

pub const SEM_POLI: u32 = u32::MAX;
pub const MEGA: u32 = u32::MAX - 1;

impl Default for CacheProp {
    fn default() -> Self {
        Self { forma: u32::MAX, deslocamento: 0, dono: Valor::UNDEFINED, forma_dono: u32::MAX, poli: SEM_POLI }
    }
}

/// Feedback de tipos (`Proto::tipos`): o OR do que cada instrução de conta ou comparação já
/// viu nos operandos, para o N2/N3 escolher o caminho rápido e as guardas (seções 8 e 11).
/// O caminho rápido (inteiro com inteiro, sem estouro) não anota, para não pesar no laço: 0 é
/// "só inteiros, ou não rodou" — numa função quente o N2 aposta em inteiro com guarda, como o
/// feedback vazio do V8. `INT` aparece quando o inteiro veio misturado com outro tipo.
pub mod tipo {
    pub const INT: u8 = 1;
    pub const DOUBLE: u8 = 2;
    pub const TEXTO: u8 = 4;
    pub const BIGINT: u8 = 8;
    /// undefined, null, booleano, símbolo, objeto.
    pub const OUTRO: u8 = 16;
    /// Entrou inteiro e o resultado não coube em 31 bits (estouro, divisão com resto).
    pub const ESTOUROU: u8 = 32;
    /// Bits 6-7: anotações seguidas sem tipo novo; com as duas ligadas (`>= ESTAVEL`) o caminho
    /// lento para de anotar. shortcut: tipo que só aparece depois disso não é anotado; o N2 o
    /// descobre pela guarda e a desotimização deve zerar o contador (reanotar).
    pub const UMA_SEM_NOVIDADE: u8 = 64;
    pub const ESTAVEL: u8 = 192;
    pub const TIPOS: u8 = 63;
}

/// Chamadas + voltas de laço/16 a partir das quais a função conta como quente (alvo do N2).
/// Só anotado: ainda não há JIT.
pub const LIMITE_QUENTE: u32 = 1000;

/// Uma função de bytecode pronta para rodar neste motor.
pub struct Proto {
    pub script: Rc<Script>,
    pub indice: u32,
    pub consts: Vec<ConstRt>,
    /// shortcut: criados na carga, não depois de algumas execuções como pede a seção 6 (função
    /// que nunca roda paga ~24 bytes por cache); criar na 1ª chamada se a memória pesar.
    pub caches: Vec<CacheProp>,
    /// Feedback de tipos por cache ([`tipo`]); os caches de propriedade/chamada deixam 0.
    pub tipos: Vec<u8>,
    pub nome: Valor,
    /// Chamadas e voltas de laço (amostradas de 256 em 256 no `SaltaAtras`): o calor para o N2
    /// ([`Proto::quente`]).
    pub chamadas: u32,
    pub voltas: u32,
    /// Fonte do script (para `Function.prototype.toString`).
    pub fonte: Option<Rc<str>>,
    /// Nome da fonte na pilha dos erros (`macro`, `api`, a URL do script…).
    pub origem: Option<Rc<str>>,
    /// Módulo dono (índice em `Motor::modulos`; `u32::MAX` = script comum).
    pub modulo: u32,
}

impl Proto {
    pub fn quente(&self) -> bool {
        self.chamadas.saturating_add(self.voltas / 16) >= LIMITE_QUENTE
    }

    #[inline]
    pub fn codigo(&self) -> &falcao_bytecode::FuncaoBytecode {
        &self.script.funcoes[self.indice as usize]
    }
}

impl Motor {
    /// O perfil de cada função que rodou (chamadas, voltas, tipos e formas vistos por cache),
    /// para depurar e para o N2: `FALCAO_PERFIL=1` no exemplo `rodar`.
    pub fn relatorio_perfil(&mut self) -> String {
        let nomes_tipo = [(tipo::INT, "int"), (tipo::DOUBLE, "double"), (tipo::TEXTO, "texto"), (tipo::BIGINT, "bigint"), (tipo::OUTRO, "outro"), (tipo::ESTOUROU, "estourou")];
        let mut s = String::new();
        for i in 0..self.protos.len() {
            let (chamadas, voltas, quente, nome) = {
                let p = &self.protos[i];
                (p.chamadas, p.voltas, p.quente(), p.nome)
            };
            if chamadas == 0 && voltas == 0 {
                continue;
            }
            let nome = self.texto_rust(nome);
            let nome = if nome.is_empty() { "<anônima>" } else { &nome };
            s += &format!("{nome}: {chamadas} chamadas, {voltas} voltas{}\n", if quente { ", QUENTE" } else { "" });
            let p = &self.protos[i];
            for (c, (&t, cp)) in p.tipos.iter().zip(&p.caches).enumerate() {
                let t = t & tipo::TIPOS;
                if t != 0 {
                    let vistos: Vec<&str> = nomes_tipo.iter().filter(|(b, _)| t & b != 0).map(|(_, n)| *n).collect();
                    s += &format!("  c{c}: {}\n", vistos.join("|"));
                } else if cp.forma != u32::MAX {
                    let estado = match cp.poli {
                        SEM_POLI => "monomórfico".to_owned(),
                        MEGA => "megamórfico".to_owned(),
                        k => format!("polimórfico ({} formas)", 1 + self.poli[k as usize].iter().filter(|e| e.forma != u32::MAX).count()),
                    };
                    s += &format!("  c{c}: propriedade {estado}\n");
                }
            }
        }
        s
    }

    // ---------------------------------------------------------------- argumentos

    #[inline]
    pub fn arg(&self, a: Args, i: u32) -> Valor {
        if i < a.n {
            self.pilha[(a.base + i) as usize]
        } else {
            Valor::UNDEFINED
        }
    }

    // ---------------------------------------------------------------- carregar

    /// Carrega um script compilado: traduz nomes e textos e devolve o protótipo da função 0.
    pub fn carregar(&mut self, script: Script) -> u32 {
        let script = Rc::new(script);
        let base = self.protos.len() as u32;
        for i in 0..script.funcoes.len() as u32 {
            let consts = script.funcoes[i as usize]
                .constantes
                .iter()
                .map(|c| match c {
                    Constante::Numero(n) => ConstRt::Valor(self.numero_permanente(*n)),
                    Constante::Texto(t) => {
                        let u = &script.textos[*t as usize];
                        ConstRt::Valor(self.texto_u16_permanente(u))
                    }
                    Constante::Nome(t) => ConstRt::Nome(self.nomes.internar(&script.textos[*t as usize])),
                    Constante::Funcao(f) => ConstRt::Funcao(base + f),
                    Constante::Classe { construtor, derivada, nome } => ConstRt::Classe {
                        construtor: if *construtor == u32::MAX { u32::MAX } else { base + construtor },
                        derivada: *derivada,
                        nome: match nome {
                            Some(t) => {
                                let c = self.nomes.internar(&script.textos[*t as usize]);
                                self.texto_do_nome(c)
                            }
                            None => Valor::VAZIA,
                        },
                    },
                    Constante::Template { .. } => ConstRt::Template(u32::MAX),
                    Constante::Regex { corpo, .. } => {
                        let u = &script.textos[*corpo as usize];
                        ConstRt::Valor(self.texto_u16_permanente(u))
                    }
                    Constante::BigInt(t) => {
                        let s = String::from_utf16_lossy(&script.textos[*t as usize]);
                        let x = crate::grande::Grande::de_digitos(s.as_bytes(), 10).unwrap_or_default();
                        ConstRt::Valor(self.gravar_bigint(&x, true))
                    }
                })
                .collect();
            let fb = &script.funcoes[i as usize];
            let nome = match fb.nome {
                Some(t) => {
                    let c = self.nomes.internar(&script.textos[t as usize]);
                    self.texto_do_nome(c)
                }
                None => Valor::VAZIA,
            };
            let caches = vec![CacheProp::default(); fb.caches as usize];
            let tipos = vec![0; fb.caches as usize];
            self.protos.push(Proto { script: script.clone(), indice: i, consts, caches, tipos, nome, chamadas: 0, voltas: 0, fonte: None, origem: None, modulo: u32::MAX });
        }
        base
    }

    /// Texto constante do bytecode: os curtos viram nome internado (compartilhado entre
    /// scripts); os longos ficam na geração velha (não somem nem mudam de lugar no berçário).
    fn texto_u16_permanente(&mut self, u: &[u16]) -> Valor {
        if u.len() <= 64 {
            let c = self.nomes.internar(u);
            return self.texto_do_nome(c);
        }
        if u.iter().all(|&c| c < 256) {
            let n = u.len() as u32;
            let d = self.g.alocar_velho(16 + n);
            self.g.por_u32(d, crate::formas::LATIN1.0);
            self.g.por_u32(d + 4, n);
            let b: Vec<u8> = u.iter().map(|&c| c as u8).collect();
            self.g.bytes_mut(d + 16, n).copy_from_slice(&b);
            return Valor::de_deslocamento(d);
        }
        let n = u.len() as u32;
        let d = self.g.alocar_velho(16 + 2 * n);
        self.g.por_u32(d, crate::formas::UTF16.0);
        self.g.por_u32(d + 4, n);
        self.g.u16s_mut(d + 16, n).copy_from_slice(u);
        Valor::de_deslocamento(d)
    }

    /// Número que não muda de lugar (constantes do bytecode).
    fn numero_permanente(&mut self, x: f64) -> Valor {
        let i = x as i32;
        if i as f64 == x && (i != 0 || x.is_sign_positive()) && (crate::valor::INT_MIN..=crate::valor::INT_MAX).contains(&i) {
            return Valor::int(i);
        }
        let d = self.g.alocar_velho(16);
        self.g.por_u32(d, crate::formas::NUMERO.0);
        self.g.por_f64(d + 8, x);
        Valor::de_deslocamento(d)
    }

    // ---------------------------------------------------------------- criar funções

    /// Objeto função com a forma dada, `length` e `name` já postos.
    pub(crate) fn objeto_funcao(&mut self, forma: FormaId, codigo: u32, contexto: Valor, comprimento: u32, nome: Valor) -> Valor {
        let f = self.novo_objeto(forma);
        self.por_interno(f, F_CODIGO, codigo);
        self.por_interno_valor(f, F_CONTEXTO, contexto);
        // Os 2 ou 3 primeiros campos da forma são length, name (e prototype).
        self.gravar_campo(f, 0, Valor::int(comprimento as i32));
        self.gravar_campo(f, 1, nome);
        if self.formas.get(forma).n > 2 {
            self.gravar_campo(f, 2, Valor::BURACO);
        }
        f
    }

    /// Função nativa nova.
    pub fn nativa(&mut self, nome: &'static str, comprimento: u32, f: FnNativa, construtor: bool) -> Valor {
        let id = self.nativas.len() as u32;
        self.nativas.push(Nativa { f, nome, construtor });
        let n = self.texto(nome);
        let forma = self.intr.forma_funcao_simples;
        self.objeto_funcao(forma, id | BIT_NATIVA, Valor::UNDEFINED, comprimento, n)
    }

    /// Registra uma nativa sem criar objeto (para criar vários depois com `nativa_com`).
    pub fn registrar_nativa(&mut self, nome: &'static str, f: FnNativa, construtor: bool) -> u32 {
        self.nativas.push(Nativa { f, nome, construtor });
        self.nativas.len() as u32 - 1
    }

    /// Número da nativa pelo nome interno (registra na primeira vez).
    pub fn id_nativa(&mut self, chave: &'static str, f: FnNativa) -> u32 {
        if let Some(&i) = self.ids_nativas.get(chave) {
            return i;
        }
        let i = self.registrar_nativa("", f, false);
        self.ids_nativas.insert(chave, i);
        i
    }

    /// Objeto função da nativa `id` com um valor capturado (lido com [`Motor::capturado`]).
    pub fn nativa_com(&mut self, id: u32, comprimento: u32, captura: Valor) -> Valor {
        let forma = self.intr.forma_funcao_simples;
        let f = self.objeto_funcao(forma, id | BIT_NATIVA, captura, comprimento, Valor::VAZIA);
        f
    }

    /// O valor capturado da nativa que está rodando.
    pub fn capturado(&self) -> Valor {
        Valor(self.interno(self.chamado, F_CONTEXTO))
    }

    /// Define `obj.nome = nativa` como método (não enumerável).
    pub fn metodo(&mut self, obj: Valor, nome: &'static str, comprimento: u32, f: FnNativa) -> Valor {
        let fun = self.nativa(nome, comprimento, f, false);
        let c = self.nomes.internar_str(nome);
        self.definir_propria(obj, c, fun, attr::METODO);
        fun
    }

    /// Fecha o protótipo `proto` sobre o contexto `ctx` (instrução `CriaFuncao`).
    pub fn closure(&mut self, proto: u32, ctx: Valor, this_seta: Valor, nome: Option<Valor>) -> Valor {
        let p = &self.protos[proto as usize];
        let fb = p.codigo();
        let fl = fb.flags;
        let comprimento = fb.comprimento;
        let nome = nome.unwrap_or(p.nome);
        let forma = if fl.gerador && fl.assincrona {
            self.intr.forma_funcao_gerador_async
        } else if fl.gerador {
            self.intr.forma_funcao_gerador
        } else if fl.assincrona {
            self.intr.forma_funcao_async
        } else if fl.sem_construtor {
            self.intr.forma_funcao_simples
        } else {
            self.intr.forma_funcao
        };
        let f = self.objeto_funcao(forma, proto, ctx, comprimento, nome);
        if fl.seta {
            self.por_interno_valor(f, F_THIS, this_seta);
        }
        f
    }

    #[inline]
    pub fn e_funcao(&self, v: Valor) -> bool {
        self.e_objeto(v) && matches!(self.formas.get(self.forma(v)).classe, Classe::Funcao | Classe::FuncaoLigada)
            || (self.e_objeto(v) && self.formas.get(self.forma(v)).classe == Classe::Proxy && self.e_funcao(Valor(self.interno(v, 0))))
    }

    pub fn e_construtor(&self, mut v: Valor) -> bool {
        if !self.e_objeto(v) {
            return false;
        }
        // `bind` de `bind` de …: desce até a função de verdade num laço.
        while self.formas.get(self.forma(v)).classe == Classe::FuncaoLigada {
            v = Valor(self.interno(v, 0));
        }
        match self.formas.get(self.forma(v)).classe {
            Classe::Funcao => {
                let c = self.interno(v, F_CODIGO);
                if c & BIT_NATIVA != 0 {
                    self.nativas[(c & !BIT_NATIVA) as usize].construtor
                } else {
                    !self.protos[c as usize].codigo().flags.sem_construtor
                }
            }
            _ => false,
        }
    }

    /// Cria o `prototype` preguiçoso de uma função na primeira leitura.
    pub fn materializar_prototype(&mut self, f: Valor) -> Valor {
        let ff = self.forma(f);
        let p = if ff == self.intr.forma_funcao_gerador {
            let forma = self.intr.forma_objeto_gerador;
            self.novo_objeto(forma)
        } else if ff == self.intr.forma_funcao_gerador_async {
            let forma = self.intr.forma_objeto_gerador_async;
            self.novo_objeto(forma)
        } else {
            let forma = self.intr.forma_objeto;
            let p = self.novo_objeto(forma);
            self.definir_propria(p, fixos::CONSTRUCTOR, f, attr::METODO);
            p
        };
        let d = self.propria(f, fixos::PROTOTYPE).expect("função sem prototype");
        self.gravar_campo(f, d.slot, p);
        p
    }

    /// Raiz de objetos comuns com o protótipo `proto` e `inline` campos.
    pub fn forma_para(&mut self, classe: Classe, proto: Valor, inline: u32) -> FormaId {
        let hp = if proto == Valor::NULL {
            0
        } else {
            let fp = self.forma(proto);
            self.formas.get_mut(fp).e_prototipo = true;
            self.formas.get(fp).hash
        };
        self.formas.raiz(classe, proto, inline, TipoElementos::Inteiros, hp)
    }
}
