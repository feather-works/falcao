//! Formas ocultas (`ARQUITETURA.md`, seção 8). Objetos criados do mesmo jeito compartilham a
//! forma: ela guarda o tipo (layout para o coletor), o protótipo, o tipo dos elementos e onde
//! mora cada propriedade. O objeto guarda só a forma e os valores.
//!
//! - Acrescentar propriedade segue uma **transição** `F --(+x)--> F'`, criada uma vez.
//! - A tabela de descritores é **compartilhada** ao longo de uma cadeia de transições: a forma
//!   com `n` propriedades usa os `n` primeiros itens da tabela.
//! - Cada forma tem uma **identidade estável** (`hash`): o hash do caminho desde a raiz, igual
//!   entre sessões. É o que o perfil e o código em disco usam.
//! - Objeto com muitas remoções vira **dicionário**: forma só dele, mudada no lugar.
//!
//! As formas moram fora da gaiola (num vetor do Rust) e nunca se movem; o número da forma é a
//! palavra 0 de todo objeto. Os protótipos guardados aqui são raízes do coletor.

use crate::valor::Valor;
use falcao_sintaxe::atomos::MapaRapido;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct FormaId(pub u32);

/// Layout do objeto na gaiola (o que o coletor precisa saber).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Tipo {
    /// `undefined`, `null`, `true`, `false`, buraco: `[forma][id]`.
    Fixo,
    /// `double` em caixa: `[forma][0][f64]`.
    Numero,
    /// Texto de 1 byte por caractere: `[forma][len][hash][átomo+1][bytes…]`.
    Latin1,
    /// Texto UTF-16: `[forma][len][hash][átomo+1][u16…]`.
    Utf16,
    /// Concatenação preguiçosa: `[forma][len][hash][0][esq][dir]`.
    Corda,
    /// Vetor de valores: `[forma][len][valores…]`.
    Lista,
    /// Vetor de `double`: `[forma][len][f64…]`.
    ListaDouble,
    /// Bytes crus: `[forma][len][bytes…]`.
    Bytes,
    /// `[forma][id][descrição]`.
    Simbolo,
    /// `[forma][len|sinal<<31][u32…]`.
    BigInt,
    /// Par de acessores: `[forma][get][set]`.
    Acessor,
    /// Contexto de variáveis capturadas: `[forma][len][pai][valores…]`.
    Contexto,
    /// Objeto do JS: `[forma][extras][elementos][internos…][campos na linha…]`.
    Objeto,
}

/// Classe do objeto JS (decide os campos internos e o comportamento exótico).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Classe {
    Comum,
    Array,
    /// Função com bytecode ou nativa.
    Funcao,
    FuncaoLigada,
    Erro,
    Caixa,
    Data,
    Regex,
    Promessa,
    Mapa,
    Conjunto,
    MapaFraco,
    ConjuntoFraco,
    RefFraca,
    Proxy,
    Argumentos,
    Gerador,
    ArrayBuffer,
    ArrayTipado,
    DataView,
    /// Iteradores nativos (de array, de mapa, de texto...).
    Iterador,
    /// Namespace de módulo.
    Modulo,
}

impl Classe {
    /// Quantos campos internos o objeto tem (entre `elementos` e os campos na linha) e quais
    /// deles são valores (bit i = 1); os outros são números crus de 32 bits.
    pub const fn internos(self) -> (u32, u32) {
        match self {
            Classe::Comum => (0, 0),
            // [length cru]
            Classe::Array => (1, 0b0),
            // [código cru][contexto][casa (objeto do método)][this/novo alvo da seta]
            Classe::Funcao => (4, 0b1110),
            // [alvo][this][argumentos (lista)]
            Classe::FuncaoLigada => (3, 0b111),
            // [pilha (texto)]
            Classe::Erro => (1, 0b1),
            // [valor primitivo]
            Classe::Caixa => (1, 0b1),
            // [tempo f64 em caixa]
            Classe::Data => (1, 0b1),
            // [fonte][flags cru][programa cru][lastIndex fica como propriedade]
            Classe::Regex => (3, 0b001),
            // [estado cru][resultado][reações (lista)][tratada cru]
            Classe::Promessa => (4, 0b0110),
            // [tabela (lista)][contagem cru]
            Classe::Mapa | Classe::Conjunto => (2, 0b01),
            Classe::MapaFraco | Classe::ConjuntoFraco => (2, 0b00),
            Classe::RefFraca => (1, 0b0),
            // [alvo][tratador]
            Classe::Proxy => (2, 0b11),
            // [contexto mapeado ou lista]
            Classe::Argumentos => (1, 0b1),
            // [estado cru][quadro salvo (lista)][função]
            Classe::Gerador => (3, 0b110),
            // [bytes][desligado cru]
            Classe::ArrayBuffer => (2, 0b01),
            // [buffer][tipo cru][deslocamento cru][comprimento cru]
            Classe::ArrayTipado | Classe::DataView => (4, 0b0001),
            // [alvo][índice cru][tipo cru]
            Classe::Iterador => (3, 0b001),
            Classe::Modulo => (1, 0b1),
        }
    }
}

/// Como os itens indexados (`[0]`, `[1]`…) estão guardados.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum TipoElementos {
    /// Inteiros pequenos, sem buracos.
    Inteiros,
    Doubles,
    Valores,
    InteirosComBuracos,
    DoublesComBuracos,
    ValoresComBuracos,
    /// Índices esparsos, numa tabela de chave → valor.
    Dicionario,
}

impl TipoElementos {
    pub fn com_buracos(self) -> bool {
        matches!(self, Self::InteirosComBuracos | Self::DoublesComBuracos | Self::ValoresComBuracos)
    }

    pub fn doubles(self) -> bool {
        matches!(self, Self::Doubles | Self::DoublesComBuracos)
    }
}

/// Chave de propriedade: átomo (nome internado) ou símbolo (bit 31 ligado).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct Chave(pub u32);

impl Chave {
    pub const BIT_SIMBOLO: u32 = 1 << 31;

    pub fn atomo(a: falcao_sintaxe::Atomo) -> Chave {
        Chave(a.0)
    }

    pub fn simbolo(id: u32) -> Chave {
        Chave(id | Self::BIT_SIMBOLO)
    }

    pub fn e_simbolo(self) -> bool {
        self.0 & Self::BIT_SIMBOLO != 0
    }
}

/// Atributos de uma propriedade.
pub mod attr {
    pub const GRAVAVEL: u8 = 1;
    pub const ENUMERAVEL: u8 = 2;
    pub const CONFIGURAVEL: u8 = 4;
    /// O campo guarda um par de acessores (`Tipo::Acessor`).
    pub const ACESSOR: u8 = 8;
    /// Valor criado só na primeira leitura (ex.: `prototype` de função).
    pub const PREGUICOSO: u8 = 16;
    /// Propriedade comum de objeto: gravável, enumerável, configurável.
    pub const PADRAO: u8 = GRAVAVEL | ENUMERAVEL | CONFIGURAVEL;
    /// Métodos de builtins e de classes: não enumerável.
    pub const METODO: u8 = GRAVAVEL | CONFIGURAVEL;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Descritor {
    pub chave: Chave,
    pub attrs: u8,
    /// Campo onde mora o valor: `< inline` = campo na linha; senão `extras[slot - inline]`.
    pub slot: u32,
}

#[derive(Default)]
struct Tabela {
    itens: Vec<Descritor>,
    /// Índice chave → posição, criado quando a tabela passa de 8 itens.
    indice: Option<MapaRapido<Chave, u32>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Passo {
    Prop(Chave, u8),
    Elementos(TipoElementos),
    /// Fechar para extensão (`preventExtensions`/`seal`/`freeze`).
    Fechar,
}

#[derive(Default)]
enum Transicoes {
    #[default]
    Nenhuma,
    Uma(Passo, FormaId),
    Muitas(MapaRapido<Passo, FormaId>),
}

pub struct Forma {
    pub tipo: Tipo,
    pub classe: Classe,
    pub elementos: TipoElementos,
    pub proto: Valor,
    /// Quantas propriedades (os `n` primeiros itens da tabela).
    pub n: u32,
    /// Campos na linha reservados no objeto.
    pub inline: u32,
    /// Próximo campo livre (igual a `n`, menos em dicionário com remoções).
    pub campos: u32,
    pub extensivel: bool,
    pub dicionario: bool,
    /// Usada como protótipo de alguém (os caches de protótipo conferem a forma dela).
    pub e_prototipo: bool,
    pub pai: Option<FormaId>,
    pub hash: u64,
    tabela: u32,
    transicoes: Transicoes,
}

/// Formas fixas dos tipos internos, nesta ordem.
pub const FIXO: FormaId = FormaId(0);
pub const NUMERO: FormaId = FormaId(1);
pub const LATIN1: FormaId = FormaId(2);
pub const UTF16: FormaId = FormaId(3);
pub const CORDA: FormaId = FormaId(4);
pub const LISTA: FormaId = FormaId(5);
pub const LISTA_DOUBLE: FormaId = FormaId(6);
pub const BYTES: FormaId = FormaId(7);
pub const SIMBOLO: FormaId = FormaId(8);
pub const BIGINT: FormaId = FormaId(9);
pub const ACESSOR: FormaId = FormaId(10);
pub const CONTEXTO: FormaId = FormaId(11);

const TIPOS_FIXOS: [Tipo; 12] = [
    Tipo::Fixo, Tipo::Numero, Tipo::Latin1, Tipo::Utf16, Tipo::Corda, Tipo::Lista,
    Tipo::ListaDouble, Tipo::Bytes, Tipo::Simbolo, Tipo::BigInt, Tipo::Acessor, Tipo::Contexto,
];

/// Mistura de 64 bits para o hash estável (determinística entre sessões).
fn misturar(h: u64, v: u64) -> u64 {
    let x = (h ^ v).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^ (x >> 29)
}

pub struct Formas {
    formas: Vec<Forma>,
    tabelas: Vec<Tabela>,
    /// Raízes de objeto por (classe, protótipo, campos na linha, elementos).
    raizes: MapaRapido<(Classe, Valor, u32, TipoElementos), FormaId>,
}

impl Default for Formas {
    fn default() -> Self {
        Self::nova()
    }
}

impl Formas {
    pub fn nova() -> Self {
        let mut f = Self { formas: Vec::with_capacity(4096), tabelas: Vec::with_capacity(1024), raizes: MapaRapido::default() };
        for (i, &tipo) in TIPOS_FIXOS.iter().enumerate() {
            f.tabelas.push(Tabela::default());
            f.formas.push(Forma {
                tipo,
                classe: Classe::Comum,
                elementos: TipoElementos::Valores,
                proto: Valor::NULL,
                n: 0,
                inline: 0,
                campos: 0,
                extensivel: false,
                dicionario: false,
                e_prototipo: false,
                pai: None,
                hash: misturar(0, i as u64),
                tabela: i as u32,
                transicoes: Transicoes::Nenhuma,
            });
        }
        f
    }

    #[inline(always)]
    pub fn get(&self, id: FormaId) -> &Forma {
        &self.formas[id.0 as usize]
    }

    #[inline(always)]
    pub fn get_mut(&mut self, id: FormaId) -> &mut Forma {
        &mut self.formas[id.0 as usize]
    }

    pub fn len(&self) -> usize {
        self.formas.len()
    }

    pub fn is_empty(&self) -> bool {
        self.formas.is_empty()
    }

    /// Forma raiz (sem propriedades) para objetos de `classe` com protótipo `proto`.
    /// `hash_proto`: identidade estável do protótipo (o hash da forma dele, ou um número fixo
    /// para os protótipos da biblioteca padrão).
    pub fn raiz(&mut self, classe: Classe, proto: Valor, inline: u32, elementos: TipoElementos, hash_proto: u64) -> FormaId {
        let chave = (classe, proto, inline, elementos);
        if let Some(&f) = self.raizes.get(&chave) {
            return f;
        }
        let id = self.nova_forma(Forma {
            tipo: Tipo::Objeto,
            classe,
            elementos,
            proto,
            n: 0,
            inline,
            campos: 0,
            extensivel: true,
            dicionario: false,
            e_prototipo: false,
            pai: None,
            hash: misturar(misturar(misturar(hash_proto, classe as u64 + 100), inline as u64), elementos as u64),
            tabela: u32::MAX,
            transicoes: Transicoes::Nenhuma,
        });
        let t = self.tabelas.len() as u32;
        self.tabelas.push(Tabela::default());
        self.formas[id.0 as usize].tabela = t;
        self.raizes.insert(chave, id);
        id
    }

    fn nova_forma(&mut self, f: Forma) -> FormaId {
        let id = FormaId(self.formas.len() as u32);
        self.formas.push(f);
        id
    }

    /// Descritores da forma (os `n` primeiros da tabela).
    pub fn descritores(&self, id: FormaId) -> &[Descritor] {
        let f = &self.formas[id.0 as usize];
        &self.tabelas[f.tabela as usize].itens[..f.n as usize]
    }

    /// Posição da propriedade `chave` na forma.
    #[inline]
    pub fn procurar(&self, id: FormaId, chave: Chave) -> Option<Descritor> {
        let f = &self.formas[id.0 as usize];
        let t = &self.tabelas[f.tabela as usize];
        if let Some(indice) = &t.indice {
            let &i = indice.get(&chave)?;
            return (i < f.n).then(|| t.itens[i as usize]);
        }
        t.itens[..f.n as usize].iter().find(|d| d.chave == chave).copied()
    }

    fn transicao(&self, de: FormaId, passo: Passo) -> Option<FormaId> {
        match &self.formas[de.0 as usize].transicoes {
            Transicoes::Nenhuma => None,
            Transicoes::Uma(p, f) => (*p == passo).then_some(*f),
            Transicoes::Muitas(m) => m.get(&passo).copied(),
        }
    }

    fn guardar_transicao(&mut self, de: FormaId, passo: Passo, para: FormaId) {
        let t = &mut self.formas[de.0 as usize].transicoes;
        match std::mem::take(t) {
            Transicoes::Nenhuma => *t = Transicoes::Uma(passo, para),
            Transicoes::Uma(p, f) => {
                let mut m = MapaRapido::default();
                m.insert(p, f);
                m.insert(passo, para);
                *t = Transicoes::Muitas(m);
            }
            Transicoes::Muitas(mut m) => {
                m.insert(passo, para);
                *t = Transicoes::Muitas(m);
            }
        }
    }

    /// Forma com a propriedade `chave` a mais (no fim). `hash_chave`: hash estável do texto da
    /// chave. Em dicionário, muda a própria forma e devolve ela mesma.
    pub fn adicionar(&mut self, de: FormaId, chave: Chave, attrs: u8, hash_chave: u64) -> FormaId {
        debug_assert!(self.procurar(de, chave).is_none());
        if self.formas[de.0 as usize].dicionario {
            let f = &mut self.formas[de.0 as usize];
            let slot = f.campos;
            f.campos += 1;
            f.n += 1;
            let t = f.tabela as usize;
            self.empurrar(t, Descritor { chave, attrs, slot });
            return de;
        }
        let passo = Passo::Prop(chave, attrs);
        if let Some(f) = self.transicao(de, passo) {
            return f;
        }
        let pai = &self.formas[de.0 as usize];
        let (n, slot, tabela_pai) = (pai.n, pai.campos, pai.tabela as usize);
        // Compartilha a tabela se o pai é o dono do fim dela; senão copia o prefixo.
        let tabela = if self.tabelas[tabela_pai].itens.len() == n as usize {
            tabela_pai
        } else {
            let itens = self.tabelas[tabela_pai].itens[..n as usize].to_vec();
            self.tabelas.push(Tabela { itens, indice: None });
            let t = self.tabelas.len() - 1;
            if n > 8 {
                self.reindexar(t);
            }
            t
        };
        self.empurrar(tabela, Descritor { chave, attrs, slot });
        let pai = &self.formas[de.0 as usize];
        let nova = Forma {
            tipo: pai.tipo,
            classe: pai.classe,
            elementos: pai.elementos,
            proto: pai.proto,
            n: n + 1,
            inline: pai.inline,
            campos: slot + 1,
            extensivel: pai.extensivel,
            dicionario: false,
            e_prototipo: false,
            pai: Some(de),
            hash: misturar(misturar(pai.hash, hash_chave), attrs as u64),
            tabela: tabela as u32,
            transicoes: Transicoes::Nenhuma,
        };
        let id = self.nova_forma(nova);
        self.guardar_transicao(de, passo, id);
        id
    }

    fn empurrar(&mut self, t: usize, d: Descritor) {
        let tab = &mut self.tabelas[t];
        tab.itens.push(d);
        let i = tab.itens.len() as u32 - 1;
        match &mut tab.indice {
            Some(ind) => {
                ind.insert(d.chave, i);
            }
            None if tab.itens.len() > 8 => self.reindexar(t),
            None => {}
        }
    }

    fn reindexar(&mut self, t: usize) {
        let tab = &mut self.tabelas[t];
        let mut m = MapaRapido::default();
        m.reserve(tab.itens.len());
        for (i, d) in tab.itens.iter().enumerate() {
            m.insert(d.chave, i as u32);
        }
        tab.indice = Some(m);
    }

    /// Forma com outro tipo de elementos (mesmas propriedades).
    pub fn com_elementos(&mut self, de: FormaId, elementos: TipoElementos) -> FormaId {
        if self.formas[de.0 as usize].elementos == elementos {
            return de;
        }
        if self.formas[de.0 as usize].dicionario {
            self.formas[de.0 as usize].elementos = elementos;
            return de;
        }
        let passo = Passo::Elementos(elementos);
        if let Some(f) = self.transicao(de, passo) {
            return f;
        }
        let id = self.derivar(de, |f| f.elementos = elementos, elementos as u64 + 1000);
        self.guardar_transicao(de, passo, id);
        id
    }

    /// Forma fechada para extensão.
    pub fn fechar(&mut self, de: FormaId) -> FormaId {
        if !self.formas[de.0 as usize].extensivel {
            return de;
        }
        if self.formas[de.0 as usize].dicionario {
            self.formas[de.0 as usize].extensivel = false;
            return de;
        }
        if let Some(f) = self.transicao(de, Passo::Fechar) {
            return f;
        }
        let id = self.derivar(de, |f| f.extensivel = false, 2000);
        self.guardar_transicao(de, Passo::Fechar, id);
        id
    }

    /// Cópia da forma (mesma tabela e propriedades) com uma mudança.
    fn derivar(&mut self, de: FormaId, mudar: impl FnOnce(&mut Forma), marca: u64) -> FormaId {
        let p = &self.formas[de.0 as usize];
        let mut nova = Forma {
            tipo: p.tipo,
            classe: p.classe,
            elementos: p.elementos,
            proto: p.proto,
            n: p.n,
            inline: p.inline,
            campos: p.campos,
            extensivel: p.extensivel,
            dicionario: false,
            e_prototipo: false,
            pai: Some(de),
            hash: misturar(p.hash, marca),
            tabela: p.tabela,
            transicoes: Transicoes::Nenhuma,
        };
        mudar(&mut nova);
        // A tabela continua do pai: quem acrescentar primeiro fica com o fim dela, o outro
        // copia o prefixo (a regra de `adicionar`).
        let id = self.nova_forma(nova);
        id
    }

    /// Forma de dicionário (só deste objeto), com as mesmas propriedades e campos.
    pub fn para_dicionario(&mut self, de: FormaId) -> FormaId {
        if self.formas[de.0 as usize].dicionario {
            return de;
        }
        let itens = self.descritores(de).to_vec();
        self.tabelas.push(Tabela { itens, indice: None });
        let t = self.tabelas.len() - 1;
        self.reindexar(t);
        let id = self.derivar(de, |f| f.dicionario = true, 3000);
        let f = &mut self.formas[id.0 as usize];
        f.tabela = t as u32;
        id
    }

    /// Remove a propriedade de uma forma de dicionário. O campo fica vago (o chamador o limpa).
    pub fn remover(&mut self, dic: FormaId, chave: Chave) -> Option<Descritor> {
        debug_assert!(self.formas[dic.0 as usize].dicionario);
        let f = &mut self.formas[dic.0 as usize];
        let t = f.tabela as usize;
        let pos = self.tabelas[t].itens.iter().position(|d| d.chave == chave)?;
        // `remove` (não `swap_remove`): a ordem de criação das chaves é visível no JS.
        let d = self.tabelas[t].itens.remove(pos);
        f.n -= 1;
        self.reindexar(t);
        Some(d)
    }

    /// Muda os atributos de uma propriedade. Forma rápida vira dicionário antes (raro).
    pub fn mudar_atributos(&mut self, de: FormaId, chave: Chave, attrs: u8) -> FormaId {
        let id = self.para_dicionario(de);
        let t = self.formas[id.0 as usize].tabela as usize;
        if let Some(d) = self.tabelas[t].itens.iter_mut().find(|d| d.chave == chave) {
            d.attrs = attrs;
        }
        id
    }

    /// Protótipos das formas e das raízes (para o coletor atualizar quando os objetos mudam de
    /// lugar). Depois de mexer, chame [`Formas::refazer_raizes`].
    pub fn protos_mut(&mut self) -> impl Iterator<Item = &mut Valor> {
        self.formas.iter_mut().map(|f| &mut f.proto)
    }

    pub fn refazer_raizes(&mut self) {
        let antigas: Vec<_> = self.raizes.drain().map(|(_, f)| f).collect();
        for f in antigas {
            let fm = &self.formas[f.0 as usize];
            self.raizes.insert((fm.classe, fm.proto, fm.inline, fm.elementos), f);
        }
    }
}

#[cfg(test)]
mod testes {
    use super::*;

    #[test]
    fn transicoes_compartilham_e_acham() {
        let mut f = Formas::nova();
        let r = f.raiz(Classe::Comum, Valor::NULL, 4, TipoElementos::Inteiros, 1);
        let (x, y) = (Chave(100), Chave(101));
        let a = f.adicionar(r, x, attr::PADRAO, 7);
        let b = f.adicionar(a, y, attr::PADRAO, 8);
        // Mesmo caminho → mesma forma.
        assert_eq!(f.adicionar(r, x, attr::PADRAO, 7), a);
        assert_eq!(f.adicionar(a, y, attr::PADRAO, 8), b);
        assert_eq!(f.procurar(b, y).unwrap().slot, 1);
        assert!(f.procurar(a, y).is_none());
        // Ramo diferente: {x, z} não enxerga y.
        let c = f.adicionar(a, Chave(102), attr::PADRAO, 9);
        assert!(f.procurar(c, y).is_none());
        assert_eq!(f.procurar(c, Chave(102)).unwrap().slot, 1);
        assert_ne!(f.get(b).hash, f.get(c).hash);
        // Muitas propriedades: índice por hash.
        let mut g = r;
        for i in 0..40 {
            g = f.adicionar(g, Chave(200 + i), attr::PADRAO, i as u64);
        }
        assert_eq!(f.procurar(g, Chave(230)).unwrap().slot, 30);
        // Dicionário: remove mantendo a ordem.
        let d = f.para_dicionario(b);
        f.remover(d, x);
        assert!(f.procurar(d, x).is_none());
        assert_eq!(f.procurar(d, y).unwrap().slot, 1);
        assert!(f.procurar(b, x).is_some(), "a forma original não muda");
    }
}
