//! Módulos ES: `import`/`export`, `import()` e `import.meta`.
//!
//! Cada módulo carregado vira um [`Modulo`]. O topo dele guarda tudo no contexto (os escopos
//! marcam os vínculos do topo como capturados), e `RegistraModulo` anota esse contexto: é de lá
//! que as exportações são lidas. A importação vira uma **célula** (lista `[contêiner,
//! deslocamento]`) resolvida na primeira leitura; ler a célula dá o valor ao vivo, e o buraco
//! é a TDZ. `export default <expressão>` mora numa lista de 1 item do próprio módulo.
//!
//! O grafo é carregado inteiro (o embutidor baixa pelo [`CarregadorModulos`]) e avaliado em
//! pós-ordem; num ciclo, o módulo que ainda está avaliando é pulado.
//!
//! shortcut: a resolução de nomes é preguiçosa (exportação inexistente só dá erro na primeira
//! leitura) e funções de um módulo do ciclo ainda não avaliado não estão prontas; módulo com
//! `await` no topo roda as microtarefas uma vez e segue mesmo se ainda estiver esperando.
//! Upgrade: ligação completa antes de avaliar (spec 16.2.1.5) e avaliação assíncrona do grafo.

use std::rc::Rc;

use crate::formas::{attr, Classe};
use crate::funcoes::{Args, Res};
use crate::intrinsecos::{simb, TipoErro};
use crate::motor::Motor;
use crate::textos::ChaveProp;
use crate::valor::Valor;
use falcao_bytecode::Exportacao;

/// Quem embute o motor diz onde achar os módulos.
pub trait CarregadorModulos {
    /// Texto do módulo em `url` (já resolvida).
    fn baixar(&mut self, url: &str) -> Result<String, String>;

    /// URL do `especificador` pedido de dentro de `base`. O padrão resolve caminhos relativos e
    /// absolutos; nome solto (`'lodash'`) precisa de mapa de importação do embutidor.
    fn resolver(&mut self, base: &str, especificador: &str) -> Result<String, String> {
        juntar_url(base, especificador)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Estado {
    Carregado,
    Avaliando,
    Avaliado,
    Falhou,
}

pub struct Modulo {
    pub url: String,
    /// Protótipo do topo.
    pub proto: u32,
    /// Módulo de cada especificador (`InfoModulo::requisitos`).
    pub requisitos: Vec<u32>,
    pub estado: Estado,
    /// Contexto do topo (undefined até o módulo começar a rodar).
    pub ctx: Valor,
    /// Lista `[valor]` do `export default <expressão>`.
    pub padrao: Valor,
    pub namespace: Valor,
    pub meta: Valor,
    pub erro: Valor,
    /// Célula de cada importação (undefined = ainda não resolvida).
    pub celulas: Vec<Valor>,
}

impl Modulo {
    /// Valores que o coletor precisa ver.
    pub fn valores_mut(&mut self) -> impl Iterator<Item = &mut Valor> {
        [&mut self.ctx, &mut self.padrao, &mut self.namespace, &mut self.meta, &mut self.erro].into_iter().chain(self.celulas.iter_mut())
    }
}

/// `especificador` relativo a `base` (só `./`, `../`, `/`, `//` e URLs completas).
pub fn juntar_url(base: &str, especificador: &str) -> Result<String, String> {
    let e = especificador;
    let tem_esquema = |s: &str| {
        let mut c = s.chars();
        c.next().is_some_and(|x| x.is_ascii_alphabetic())
            && s.find(':').is_some_and(|i| s[..i].chars().all(|x| x.is_ascii_alphanumeric() || "+-.".contains(x)))
    };
    if tem_esquema(e) && !e.starts_with("./") {
        return Ok(e.to_string());
    }
    if !(e.starts_with("./") || e.starts_with("../") || e.starts_with('/')) {
        return Err(format!("Failed to resolve module specifier \"{e}\". Relative references must start with either \"/\", \"./\", or \"../\"."));
    }
    let base = base.split(['?', '#']).next().unwrap_or("");
    let (esquema, resto) = base.split_once("://").unwrap_or(("", base));
    if let Some(r) = e.strip_prefix("//") {
        return Ok(format!("{esquema}://{r}"));
    }
    let (origem, caminho) = match resto.find('/') {
        Some(i) => (&resto[..i], &resto[i..]),
        None => (resto, "/"),
    };
    let juntos = if e.starts_with('/') { e.to_string() } else { format!("{}{e}", &caminho[..caminho.rfind('/').map_or(0, |i| i + 1)]) };
    let (juntos, cauda) = match juntos.find(['?', '#']) {
        Some(i) => (juntos[..i].to_string(), juntos[i..].to_string()),
        None => (juntos, String::new()),
    };
    let mut partes: Vec<&str> = Vec::new();
    let segs: Vec<&str> = juntos.split('/').collect();
    for (i, s) in segs.iter().enumerate() {
        match *s {
            "." => {}
            ".." => {
                partes.pop();
            }
            "" if i > 0 && i + 1 < segs.len() => {}
            s => partes.push(s),
        }
    }
    let mut caminho = partes.join("/");
    if !caminho.starts_with('/') {
        caminho.insert(0, '/');
    }
    if matches!(segs.last(), Some(&".") | Some(&"..")) {
        caminho.push('/');
    }
    let prefixo = if esquema.is_empty() { origem.to_string() } else { format!("{esquema}://{origem}") };
    Ok(format!("{prefixo}{caminho}{cauda}"))
}

impl Motor {
    /// Carrega `url` (com o texto já em mãos), os módulos que ele pede e roda tudo. Devolve o
    /// namespace. Chamar no topo (nenhum `Valor` solto no Rust): o coletor pode rodar dentro.
    pub fn avaliar_modulo(&mut self, url: &str, fonte: &str) -> Res<Valor> {
        let id = self.carregar_modulo(url, Some(fonte))?;
        self.avaliar_modulo_id(id)?;
        Ok(self.namespace(id))
    }

    fn resolver_modulo(&mut self, base: &str, especificador: &str) -> Res<String> {
        let r = match self.carregador_modulos.as_mut() {
            Some(c) => c.resolver(base, especificador),
            None => juntar_url(base, especificador),
        };
        r.or_else(|e| self.erro_tipo(&e))
    }

    fn carregar_modulo(&mut self, url: &str, fonte: Option<&str>) -> Res<u32> {
        if let Some(&id) = self.modulos_por_url.get(url) {
            return match self.modulos[id as usize].estado {
                Estado::Falhou => Err(self.modulos[id as usize].erro),
                _ => Ok(id),
            };
        }
        let fonte = match fonte {
            Some(f) => f.to_string(),
            None => {
                let r = match self.carregador_modulos.as_mut() {
                    Some(c) => c.baixar(url),
                    None => Err("sem carregador de módulos".to_string()),
                };
                match r {
                    Ok(f) => f,
                    Err(e) => return self.erro_tipo(&format!("Failed to fetch dynamically imported module: {url} ({e})")),
                }
            }
        };
        let script = match falcao_sintaxe::analisar_modulo(&fonte) {
            Ok((arvore, lexico)) => {
                let esc = falcao_sintaxe::analisar_escopos(&arvore);
                match falcao_bytecode::compilar(&arvore, &esc, &lexico) {
                    Ok(s) => s,
                    Err(e) => return Err(self.erro(TipoErro::Sintaxe, &format!("{url}: {}", e.mensagem))),
                }
            }
            Err(e) => return Err(self.erro(TipoErro::Sintaxe, &format!("{url}: {}", e.mensagem))),
        };
        let n_import = script.info_modulo.importacoes.len();
        let especificadores: Vec<String> = script.info_modulo.requisitos.iter().map(|r| String::from_utf16_lossy(r)).collect();
        let id = self.modulos.len() as u32;
        let base = self.carregar(script);
        let texto: Rc<str> = Rc::from(fonte.as_str());
        for p in &mut self.protos[base as usize..] {
            p.fonte = Some(Rc::clone(&texto));
            p.modulo = id;
        }
        let padrao = self.nova_lista(1);
        self.lista_set(padrao, 0, Valor::BURACO);
        let u = Valor::UNDEFINED;
        self.modulos.push(Modulo {
            url: url.to_string(),
            proto: base,
            requisitos: Vec::new(),
            estado: Estado::Carregado,
            ctx: u,
            padrao,
            namespace: u,
            meta: u,
            erro: u,
            celulas: vec![u; n_import],
        });
        self.modulos_por_url.insert(url.to_string(), id);
        for esp in especificadores {
            let r = self.resolver_modulo(url, &esp).and_then(|u| self.carregar_modulo(&u, None));
            match r {
                Ok(r) => self.modulos[id as usize].requisitos.push(r),
                Err(e) => {
                    self.falhar_modulo(id, e);
                    return Err(e);
                }
            }
        }
        Ok(id)
    }

    fn falhar_modulo(&mut self, id: u32, e: Valor) {
        let m = &mut self.modulos[id as usize];
        m.estado = Estado::Falhou;
        m.erro = e;
    }

    /// Roda o módulo depois dos que ele pede (pós-ordem; ciclo: quem está rodando é pulado).
    fn avaliar_modulo_id(&mut self, id: u32) -> Res<()> {
        match self.modulos[id as usize].estado {
            Estado::Avaliado | Estado::Avaliando => return Ok(()),
            Estado::Falhou => return Err(self.modulos[id as usize].erro),
            Estado::Carregado => {}
        }
        self.modulos[id as usize].estado = Estado::Avaliando;
        for i in 0..self.modulos[id as usize].requisitos.len() {
            let r = self.modulos[id as usize].requisitos[i];
            if let Err(e) = self.avaliar_modulo_id(r) {
                self.falhar_modulo(id, e);
                return Err(e);
            }
        }
        let proto = self.modulos[id as usize].proto;
        let f = self.closure(proto, Valor::UNDEFINED, Valor::UNDEFINED, None);
        let mut r = self.chamar_externo(f, Valor::UNDEFINED, &[]);
        if let Ok(p) = r {
            // `await` no topo: o topo devolveu a promessa dele.
            if self.protos[proto as usize].codigo().flags.assincrona && self.e_promessa(p) {
                self.raizes.push(p);
                self.rodar_microtarefas();
                let p = self.raizes.pop().unwrap();
                if self.interno(p, 0) == crate::promessas::REJEITADA {
                    self.marcar_tratada(p);
                    r = Err(Valor(self.interno(p, 1)));
                }
            }
        }
        match r {
            Ok(_) => {
                self.modulos[id as usize].estado = Estado::Avaliado;
                Ok(())
            }
            Err(e) => {
                self.falhar_modulo(id, e);
                Err(e)
            }
        }
    }

    // ---------------------------------------------------------------- ligações

    /// Célula `[contêiner, deslocamento]`.
    fn celula(&mut self, conteiner: Valor, deslocamento: u32) -> Valor {
        let l = self.nova_lista(2);
        self.lista_set(l, 0, conteiner);
        self.lista_set(l, 1, Valor::int(deslocamento as i32));
        l
    }

    fn ler_celula(&mut self, c: Valor, nome: &[u16]) -> Res<Valor> {
        let (cont, off) = (self.lista_get(c, 0), self.lista_get(c, 1).como_int() as u32);
        let v = self.g.valor_em(cont.deslocamento() + off);
        if v == Valor::BURACO {
            let n = String::from_utf16_lossy(nome);
            return self.erro_referencia(&format!("Cannot access '{n}' before initialization"));
        }
        Ok(v)
    }

    /// Onde mora a exportação `nome` do módulo `id` (`None`: não existe).
    fn resolver_exportacao(&mut self, id: u32, nome: &[u16], vistos: &mut Vec<(u32, Vec<u16>)>) -> Res<Option<Valor>> {
        if vistos.iter().any(|(i, n)| *i == id && n == nome) {
            return Ok(None);
        }
        vistos.push((id, nome.to_vec()));
        let script = Rc::clone(&self.protos[self.modulos[id as usize].proto as usize].script);
        let info = &script.info_modulo;
        if let Some((_, e)) = info.exportacoes.iter().find(|(n, _)| n == nome) {
            let m = &self.modulos[id as usize];
            return match e {
                Exportacao::Local(slot) => {
                    let ctx = m.ctx;
                    if !ctx.e_ref() {
                        let n = String::from_utf16_lossy(nome);
                        return self.erro_referencia(&format!("Cannot access '{n}' before initialization"));
                    }
                    Ok(Some(self.celula(ctx, 12 + 4 * slot)))
                }
                Exportacao::Padrao => {
                    let p = m.padrao;
                    Ok(Some(self.celula(p, 8)))
                }
                Exportacao::Indireta(req, n2) => {
                    let alvo = m.requisitos[*req as usize];
                    self.resolver_exportacao(alvo, n2, vistos)
                }
                Exportacao::Namespace(req) => {
                    let alvo = m.requisitos[*req as usize];
                    let ns = self.namespace(alvo);
                    let l = self.nova_lista(1);
                    self.lista_set(l, 0, ns);
                    Ok(Some(self.celula(l, 8)))
                }
            };
        }
        if nome == "default".encode_utf16().collect::<Vec<_>>().as_slice() {
            return Ok(None);
        }
        for &r in &info.estrelas {
            let alvo = self.modulos[id as usize].requisitos[r as usize];
            if let Some(c) = self.resolver_exportacao(alvo, nome, vistos)? {
                return Ok(Some(c));
            }
        }
        Ok(None)
    }

    /// `LeImport k` no protótipo `proto`.
    pub(crate) fn ler_import(&mut self, proto: u32, k: u32) -> Res<Valor> {
        let id = self.protos[proto as usize].modulo;
        let script = Rc::clone(&self.protos[proto as usize].script);
        let (req, nome) = &script.info_modulo.importacoes[k as usize];
        let mut c = self.modulos[id as usize].celulas[k as usize];
        if c == Valor::UNDEFINED {
            let alvo = self.modulos[id as usize].requisitos[*req as usize];
            c = match self.resolver_exportacao(alvo, nome, &mut Vec::new())? {
                Some(c) => c,
                None => {
                    let (n, u) = (String::from_utf16_lossy(nome), self.modulos[alvo as usize].url.clone());
                    return Err(self.erro(TipoErro::Sintaxe, &format!("The requested module '{u}' does not provide an export named '{n}'")));
                }
            };
            self.modulos[id as usize].celulas[k as usize] = c;
        }
        self.ler_celula(c, nome)
    }

    /// `GuardaPadrao`.
    pub(crate) fn guardar_padrao(&mut self, proto: u32, v: Valor) {
        let id = self.protos[proto as usize].modulo;
        let p = self.modulos[id as usize].padrao;
        self.lista_set(p, 0, v);
    }

    /// `RegistraModulo`.
    pub(crate) fn registrar_contexto_modulo(&mut self, proto: u32, ctx: Valor) {
        let id = self.protos[proto as usize].modulo;
        self.modulos[id as usize].ctx = ctx;
    }

    /// `CarregaNamespace n`.
    pub(crate) fn namespace_do_requisito(&mut self, proto: u32, n: u32) -> Valor {
        let id = self.protos[proto as usize].modulo;
        let alvo = self.modulos[id as usize].requisitos[n as usize];
        self.namespace(alvo)
    }

    // ---------------------------------------------------------------- namespace

    fn nomes_exportados(&self, id: u32, saida: &mut Vec<Vec<u16>>, vistos: &mut Vec<u32>) {
        if vistos.contains(&id) {
            return;
        }
        vistos.push(id);
        let m = &self.modulos[id as usize];
        let info = &self.protos[m.proto as usize].script.info_modulo;
        let padrao: Vec<u16> = "default".encode_utf16().collect();
        let externo = vistos.len() > 1;
        for (n, _) in &info.exportacoes {
            if !(externo && *n == padrao) {
                saida.push(n.clone());
            }
        }
        for &r in &info.estrelas {
            self.nomes_exportados(m.requisitos[r as usize], saida, vistos);
        }
    }

    /// Objeto namespace (`import * as ns`, `import()`): cada exportação vira um getter ao vivo.
    pub fn namespace(&mut self, id: u32) -> Valor {
        let atual = self.modulos[id as usize].namespace;
        if atual != Valor::UNDEFINED {
            return atual;
        }
        let mut nomes = Vec::new();
        self.nomes_exportados(id, &mut nomes, &mut Vec::new());
        nomes.sort();
        nomes.dedup();
        let f = self.forma_para(Classe::Modulo, Valor::NULL, 0);
        let o = self.novo_objeto(f);
        self.por_interno_valor(o, 0, Valor::int(id as i32));
        self.modulos[id as usize].namespace = o;
        let getter = match self.ids_nativas.get("__namespace_get") {
            Some(&g) => g,
            None => {
                let g = self.registrar_nativa("get", namespace_get, false);
                self.ids_nativas.insert("__namespace_get", g);
                g
            }
        };
        for n in nomes {
            let t = self.texto_u16(&n);
            let cap = self.nova_lista(2);
            self.lista_set(cap, 0, Valor::int(id as i32));
            self.lista_set(cap, 1, t);
            let g = self.nativa_com(getter, 0, cap);
            let k = self.nomes.internar(&n);
            self.definir_acessor(o, ChaveProp::Nome(k), g, false, true);
        }
        let tag = self.texto("Module");
        self.definir_propria(o, simb::TO_STRING_TAG, tag, 0);
        let fechada = self.formas.fechar(self.forma(o));
        self.g.por_u32(o.deslocamento(), fechada.0);
        o
    }

    // ---------------------------------------------------------------- import.meta e import()

    pub(crate) fn import_meta(&mut self, proto: u32) -> Valor {
        let id = self.protos[proto as usize].modulo;
        if id == u32::MAX {
            return self.objeto_vazio();
        }
        let atual = self.modulos[id as usize].meta;
        if atual != Valor::UNDEFINED {
            return atual;
        }
        let o = self.objeto_vazio();
        let url = self.modulos[id as usize].url.clone();
        let u = self.texto(&url);
        let k = self.chave("url");
        self.definir_propria(o, k, u, attr::PADRAO);
        self.modulos[id as usize].meta = o;
        o
    }

    /// `import(especificador)` dentro do protótipo `proto`: promessa do namespace.
    pub(crate) fn importar_dinamico(&mut self, proto: u32, especificador: Valor) -> Valor {
        // Roda dentro de uma instrução: o laço de fora segura valores, então nada de coleta aqui.
        self.aninhamento += 1;
        let p = self.nova_promessa();
        let base = match self.protos[proto as usize].modulo {
            u32::MAX => self.url_base.clone(),
            id => self.modulos[id as usize].url.clone(),
        };
        let r = self.para_texto_rust(especificador).and_then(|s| {
            let url = self.resolver_modulo(&base, &s)?;
            let id = self.carregar_modulo(&url, None)?;
            self.avaliar_modulo_id(id)?;
            Ok(self.namespace(id))
        });
        match r {
            Ok(ns) => self.resolver_promessa(p, ns),
            Err(e) => self.rejeitar_promessa(p, e),
        }
        self.aninhamento -= 1;
        p
    }
}

fn namespace_get(m: &mut Motor, _this: Valor, _a: Args, _nt: Valor) -> Res<Valor> {
    let cap = m.capturado();
    let id = m.lista_get(cap, 0).como_int() as u32;
    let t = m.lista_get(cap, 1);
    let nome = m.texto_u16_de(t);
    match m.resolver_exportacao(id, &nome, &mut Vec::new())? {
        Some(c) => m.ler_celula(c, &nome),
        None => Ok(Valor::UNDEFINED),
    }
}

#[cfg(test)]
mod testes {
    use super::*;
    use std::collections::HashMap;

    struct Memoria(HashMap<String, String>);

    impl CarregadorModulos for Memoria {
        fn baixar(&mut self, url: &str) -> Result<String, String> {
            self.0.get(url).cloned().ok_or_else(|| "404".to_string())
        }
    }

    fn motor(arquivos: &[(&str, &str)]) -> Motor {
        let mut m = Motor::novo();
        m.carregador_modulos = Some(Box::new(Memoria(arquivos.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect())));
        m
    }

    fn rodar(m: &mut Motor, fonte: &str) -> String {
        let r = m.avaliar_modulo("https://x.test/app/main.js", fonte);
        m.rodar_microtarefas();
        match r {
            Ok(_) => m.avaliar("globalThis.r").map(|v| m.para_texto_rust(v).unwrap()).unwrap(),
            Err(e) => format!("ERRO {}", m.texto_do_erro(e)),
        }
    }

    #[test]
    fn juntar() {
        let b = "https://x.test/a/b/c.js?q=1";
        assert_eq!(juntar_url(b, "./d.js").unwrap(), "https://x.test/a/b/d.js");
        assert_eq!(juntar_url(b, "../d.js").unwrap(), "https://x.test/a/d.js");
        assert_eq!(juntar_url(b, "../../../d.js").unwrap(), "https://x.test/d.js");
        assert_eq!(juntar_url(b, "/e/f.js").unwrap(), "https://x.test/e/f.js");
        assert_eq!(juntar_url(b, "//cdn.test/g.js").unwrap(), "https://cdn.test/g.js");
        assert_eq!(juntar_url(b, "https://y.test/h.js").unwrap(), "https://y.test/h.js");
        assert_eq!(juntar_url("file:///home/u/p.html", "./m.js").unwrap(), "file:///home/u/m.js");
        assert!(juntar_url(b, "lodash").is_err());
    }

    #[test]
    fn importa_exporta_ao_vivo() {
        let mut m = motor(&[
            ("https://x.test/app/a.js", "export let n = 1; export function mais() { n++ } export default 'pd'; export const {x, y: [z]} = {x: 10, y: [20]};"),
            ("https://x.test/app/b.js", "export * from './a.js'; export { mais as soma } from './a.js'; export * as tudo from './a.js';"),
        ]);
        let r = rodar(
            &mut m,
            "import d, { n, mais } from './a.js'; import * as ns from './a.js'; import { soma, tudo, x, z } from './b.js';
             mais(); soma();
             globalThis.r = [d, n, ns.n, Object.keys(ns).join(), tudo.default, x, z, Object.prototype.toString.call(ns)].join('|');",
        );
        assert_eq!(r, "pd|3|3|default,mais,n,x,z|pd|10|20|[object Module]");
    }

    #[test]
    fn erros_de_modulo() {
        let mut m = motor(&[("https://x.test/app/a.js", "export const a = 1;")]);
        assert!(rodar(&mut m, "import { b } from './a.js'; globalThis.r = b").contains("does not provide an export named 'b'"));
        let mut m = motor(&[]);
        assert!(rodar(&mut m, "import './nada.js';").contains("TypeError"));
        let mut m = motor(&[("https://x.test/app/a.js", "export const a = 1;")]);
        assert!(rodar(&mut m, "import { a } from './a.js'; a = 2;").contains("Assignment to constant"));
    }

    #[test]
    fn ciclo_import_dinamico_meta_e_await() {
        let mut m = motor(&[
            ("https://x.test/app/a.js", "import { b } from './b.js'; export const a = 'A'; export function vb() { return b }"),
            ("https://x.test/app/b.js", "import { a } from './a.js'; export const b = 'B'; export function va() { return a }"),
            ("https://x.test/app/d.js", "export default class { oi() { return 'oi' } }; export const u = import.meta.url;"),
        ]);
        let r = rodar(
            &mut m,
            "import { vb } from './a.js'; import { va } from './b.js';
             const d = await import('./d.js');
             globalThis.r = [vb(), va(), new d.default().oi(), d.u.endsWith('/app/d.js'), import.meta.url].join('|');",
        );
        assert_eq!(r, "B|A|oi|true|https://x.test/app/main.js");
    }

    #[test]
    fn modulos_sobrevivem_ao_coletor() {
        let mut m = motor(&[("https://x.test/app/a.js", "export let lista = []; export function por(x) { lista.push({x}) }")]);
        m.estresse = true;
        m.g.coleta_pendente = true;
        let r = rodar(&mut m, "import { lista, por } from './a.js'; for (let i = 0; i < 5000; i++) { por(i); let lixo = [i] } globalThis.r = lista.length + ':' + lista[4999].x;");
        assert_eq!(r, "5000:4999");
        assert!(m.coletas[0] > 100);
    }
}
