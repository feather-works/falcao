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

/// Cache inline de propriedade (monomórfico; ver `ARQUITETURA.md`, seção 8).
#[derive(Clone, Copy, Debug)]
pub struct CacheProp {
    /// Forma do receptor (u32::MAX = vazio).
    pub forma: u32,
    /// Campo na linha (offset em bytes desde o objeto) ou, com `extras`, índice em extras.
    pub deslocamento: u32,
    pub extras: bool,
    /// Achado no protótipo: o objeto que tem a propriedade e a forma dele.
    pub dono: Valor,
    pub forma_dono: u32,
}

impl Default for CacheProp {
    fn default() -> Self {
        Self { forma: u32::MAX, deslocamento: 0, extras: false, dono: Valor::UNDEFINED, forma_dono: u32::MAX }
    }
}

/// Uma função de bytecode pronta para rodar neste motor.
pub struct Proto {
    pub script: Rc<Script>,
    pub indice: u32,
    pub consts: Vec<ConstRt>,
    pub caches: Vec<CacheProp>,
    pub nome: Valor,
    /// Chamadas (calor, para o N2 no futuro).
    pub chamadas: u32,
    /// Fonte do script (para `Function.prototype.toString`).
    pub fonte: Option<Rc<str>>,
    /// Nome da fonte na pilha dos erros (`macro`, `api`, a URL do script…).
    pub origem: Option<Rc<str>>,
    /// Módulo dono (índice em `Motor::modulos`; `u32::MAX` = script comum).
    pub modulo: u32,
}

impl Proto {
    #[inline]
    pub fn codigo(&self) -> &falcao_bytecode::FuncaoBytecode {
        &self.script.funcoes[self.indice as usize]
    }
}

impl Motor {
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
            self.protos.push(Proto { script: script.clone(), indice: i, consts, caches, nome, chamadas: 0, fonte: None, origem: None, modulo: u32::MAX });
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
        let forma = if fl.gerador {
            self.intr.forma_funcao_gerador
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

    pub fn e_construtor(&self, v: Valor) -> bool {
        if !self.e_objeto(v) {
            return false;
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
            Classe::FuncaoLigada => self.e_construtor(Valor(self.interno(v, 0))),
            _ => false,
        }
    }

    /// Cria o `prototype` preguiçoso de uma função na primeira leitura.
    pub fn materializar_prototype(&mut self, f: Valor) -> Valor {
        let gerador = self.forma(f) == self.intr.forma_funcao_gerador;
        let p = if gerador {
            let forma = self.intr.forma_objeto_gerador;
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
        let el = if classe == Classe::Array { TipoElementos::Inteiros } else { TipoElementos::Inteiros };
        self.formas.raiz(classe, proto, inline, el, hp)
    }
}
