"""Gera crates/falcao-sintaxe/src/unicode_tabelas.rs a partir dos dados oficiais do Unicode
(dado público do Unicode Consortium, não código de outro projeto). Uso:

    python3 -I ferramentas/gerar_unicode.py PASTA_COM_OS_TXT

A pasta precisa ter DerivedCoreProperties.txt e DerivedGeneralCategory.txt, baixados de
https://www.unicode.org/Public/UCD/latest/ucd/ (o segundo fica em extracted/).
"""
import os, re, sys

pasta = sys.argv[1]
SAIDA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "crates", "falcao-sintaxe", "src", "unicode_tabelas.rs")


def ler(arquivo, propriedade):
    faixas = []
    versao = None
    for linha in open(os.path.join(pasta, arquivo), encoding="utf-8"):
        if versao is None:
            m = re.match(r"# \w+-([\d.]+)\.txt", linha)
            if m:
                versao = m.group(1)
        linha = linha.split("#", 1)[0].strip()
        if not linha:
            continue
        cod, prop = [p.strip() for p in linha.split(";")[:2]]
        if prop != propriedade:
            continue
        a, _, b = cod.partition("..")
        faixas.append((int(a, 16), int(b or a, 16)))
    faixas.sort()
    juntas = []
    for a, b in faixas:
        if juntas and a <= juntas[-1][1] + 1:
            juntas[-1] = (juntas[-1][0], max(juntas[-1][1], b))
        else:
            juntas.append((a, b))
    return versao, juntas


v1, id_start = ler("DerivedCoreProperties.txt", "ID_Start")
_, id_continue = ler("DerivedCoreProperties.txt", "ID_Continue")
v2, zs = ler("DerivedGeneralCategory.txt", "Zs")
assert v1 == v2, (v1, v2)


def tabela(nome, faixas, doc):
    linhas = [f"/// {doc} ({len(faixas)} faixas, Unicode {v1}).", f"pub(crate) static {nome}: &[(u32, u32)] = &["]
    for i in range(0, len(faixas), 6):
        linhas.append("    " + " ".join(f"(0x{a:X}, 0x{b:X})," for a, b in faixas[i:i + 6]))
    linhas.append("];")
    return "\n".join(linhas)


texto = "\n\n".join([
    "//! Tabelas geradas por `ferramentas/gerar_unicode.py` a partir dos dados do Unicode "
    f"{v1}.\n//! Não editar à mão: rodar o gerador de novo quando sair versão nova do Unicode.",
    tabela("ID_START", id_start, "`ID_Start`: começo de identificador (ECMA-262 §12.7)"),
    tabela("ID_CONTINUE", id_continue, "`ID_Continue`: resto de identificador"),
    tabela("ESPACO_ZS", zs, "Categoria `Zs` (separador de espaço): espaço em branco do JS"),
]) + "\n"
open(SAIDA, "w", encoding="utf-8").write(texto)
print(f"Unicode {v1}: ID_Start {len(id_start)} faixas, ID_Continue {len(id_continue)}, Zs {len(zs)} -> {os.path.normpath(SAIDA)}")
