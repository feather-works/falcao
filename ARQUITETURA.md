# Falcão: como o JavaScript é compilado

Projeto da arquitetura, escrito em 18/10/2026 a pedido do usuário. Nada daqui foi
implementado ainda; as metas e as fases estão em `featherweb/docs/js/PLANO-JIT.md`. Tudo é
escrito por nós: os nomes das instruções, os formatos e os algoritmos são deste projeto.
Do V8 e do SpiderMonkey só se usa o comportamento observado de fora (regra 8 do `CLAUDE.md`).

## 1. O caminho inteiro

```
 texto do script (UTF-8, como veio da rede)
   │
   │ 1. pré-análise       só a casca das funções: onde começam/terminam, nomes de fora
   ▼
 tabela de funções (cada uma: faixa no texto, nº de parâmetros, flags)
   │                      ── o corpo só é lido na 1ª chamada ──
   │ 2. léxico            texto → símbolos (nomes, números, textos, pontuação)
   │ 3. sintaxe           símbolos → árvore (em arena, índices de 32 bits)
   │ 4. escopos           cada nome → registrador, espaço no contexto ou global
   │ 5. gerador           árvore → bytecode + constantes + nº de caches
   ▼
 bytecode (imutável; compartilhado entre abas pelo hash do script)
   │
   │ N1 intérprete        executa; caches inline anotam formas e tipos (o "perfil")
   │                      contadores de calor por função e por laço
   ▼ morna
   │ N2 JIT base          bytecode → código de máquina, uma passada, mesmo quadro do N1
   ▼ quente + perfil estável
   │ N3 JIT otimizador    bytecode + perfil → grafo SSA → otimiza → máquina
   │                      (thread auxiliar; aposta nos tipos com guardas)
   ▼
 código de máquina ──── guarda falhou ───► desotimiza: volta ao N1 no ponto certo
   │
   └─► caches em disco: pré-análise, bytecode, perfil e código (para a próxima visita)
```

Em toda a descrição abaixo, usamos esta função de exemplo:

```js
function soma(pontos) {
  let total = 0;
  for (let i = 0; i < pontos.length; i++) total += pontos[i].x;
  return total;
}
soma([{ x: 1, y: 2 }, { x: 3, y: 4 }]);
```

## 2. Pré-análise (preguiçosa)

Ao chegar um script, só a casca das funções é lida. A pré-análise:

- confere a sintaxe do arquivo inteiro (erro de sintaxe tem de aparecer na carga, como
  manda a norma);
- para cada função, guarda o **início e o fim no texto**, o número de parâmetros e as
  flags (`"use strict"`, se usa `eval`, `arguments` ou `this`, se é `async`/gerador/flecha);
- anota quais nomes a função **pega de fora** (capturas). Isso decide se as variáveis da
  função de fora ficam em registrador ou num contexto no heap.

Para `soma`, sai uma linha da tabela: `soma: texto[9..142], 1 parâmetro, sem captura,
sem eval`. Nenhuma árvore, nenhum bytecode. Funções que se chamam na hora,
`(function(){…})()`, são reconhecidas pelo parêntese de abertura e compiladas direto.

**Por quê:** os sites mandam megabytes de JS, e boa parte das funções nunca roda. Ler a
casca custa uma fração de montar a árvore.

## 3. Léxico

O texto vira uma sequência de **símbolos**. Cada símbolo tem o tipo, a posição (início e
fim em bytes, 32 bits cada) e se houve quebra de linha antes (necessário para a inserção
automática de `;`).

```
nome(function) nome(soma) ( nome(pontos) ) { nome(let) nome(total) = número(0) ; …
```

- **O léxico não decide sozinho o que é `/`**: depois de uma expressão é divisão; no
  começo de uma, é expressão regular. Quem diz qual dos dois é a sintaxe, que pede um
  símbolo de cada vez informando o "objetivo".
- Dentro de template (`` `a${x}b` ``), o `}` que fecha `${…}` faz o léxico continuar o
  template.
- Os números já saem convertidos (`0x1F`, `1_000`, `.5e3`, `10n`). Os textos saem
  com os escapes resolvidos, em UTF-16, como o JS exige (`"\uD800"` sozinho é válido).
- Caracteres de nome: `ID_Start`/`ID_Continue` do Unicode, pelas tabelas geradas
  (`crates/falcao-sintaxe/src/unicode_tabelas.rs`). ASCII vai pelo caminho rápido.

## 4. Sintaxe: a árvore

A árvore fica numa **arena**: um vetor de nós; um filho é um índice de 32 bits, não um
ponteiro. A árvore de uma função é jogada fora inteira depois de gerar o bytecode, sem
liberar nó por nó.

```
Função soma (parâmetros: [pontos])
└─ Bloco
   ├─ Declaração let  total = Número 0
   ├─ Para
   │   ├─ início:  Declaração let  i = Número 0
   │   ├─ teste:   Menor( Nome i , Membro( Nome pontos , "length" ) )
   │   ├─ passo:   Incremento-pós( Nome i )
   │   └─ corpo:   Atribuição+= ( Nome total , Membro( Índice( Nome pontos , Nome i ) , "x" ) )
   └─ Retorno( Nome total )
```

A sintaxe cobre o ECMAScript inteiro (ES2025): classes com campos privados, `async`/`await`,
geradores, desestruturação, módulos, `?.`, `??`, templates marcados e o Anexo B
(comportamentos legados que os sites ainda usam). Os erros que a norma manda dar antes de
rodar ("early errors") são checados aqui.

## 5. Escopos: onde cada variável mora

Cada nome é resolvido uma vez, na compilação:

| Nome | Onde mora | Por quê |
|---|---|---|
| `pontos` | `a0` (argumento 0, no quadro) | parâmetro sem captura |
| `total` | `r0` (registrador do quadro) | local sem captura |
| `i` | `r1` | local sem captura (o `let` do `for` ganharia uma cópia por volta só se alguma função interna o capturasse) |
| `soma` (de fora) | global, via cache inline | nome do topo do script |

Variável capturada por função interna vai para um **contexto** (um objeto pequeno no
heap, com um campo por variável), e a função interna guarda o ponteiro para ele. Variável
que ninguém captura nunca sai do quadro, então custa o mesmo que em C.

`eval` direto e `with` desligam essas otimizações só na função que os usa, que passa a
procurar nomes pelo caminho lento.

## 6. Bytecode

Máquina de **registradores + acumulador**: cada instrução lê registradores e/ou constantes
e escreve no acumulador (`acc`). Os nomes das instruções são nossos.

```
soma(a0)          registradores: r0 r1     caches: c0..c3
 0  CARREGA_INT     0            ; acc = 0
 2  GUARDA          r0           ; total = acc
 4  CARREGA_INT     0
 6  GUARDA          r1           ; i = 0
 8  LE_PROP         a0, k0, c0   ; acc = pontos.length          (k0 = "length")
12  MENOR_QUE       r1, c1       ; acc = i < acc
15  SALTA_SE_FALSO  @38
17  LE_ITEM         a0, r1, c2   ; acc = pontos[i]
21  LE_PROP_ACC     k1, c3       ; acc = acc.x                  (k1 = "x")
24  SOMA            r0, c4       ; acc = total + acc
27  GUARDA          r0
29  INCREMENTA      r1, c5       ; i++
32  SALTA_ATRAS     @8           ; volta do laço (conta calor)
35  …
38  CARREGA         r0
40  RETORNA
```

- **Codificação**: 1 byte de instrução, operandos de 1 byte; o prefixo `LARGO` dobra (ou
  quadruplica) os operandos da instrução seguinte. A maioria das funções cabe em operandos
  de 1 byte.
- **Constantes** (`k0`, `k1`…): nomes internados, números que não cabem em 31 bits,
  funções internas, modelos de objeto literal.
- **Caches** (`c0`…): cada instrução que lê ou escreve propriedade, chama função ou faz
  conta tem um índice de cache. O vetor de caches só é criado **depois de algumas
  execuções** (função que roda uma vez não paga a memória do perfil).
- **Tabela de exceções**: faixas de bytecode → onde fica o `catch`. Não há custo no
  caminho sem exceção.
- **Mapa de posições**: deslocamento do bytecode → linha/coluna no texto, comprimido, só
  consultado em erro e no F12.
- O bytecode de uma função é **imutável**: pode ser compartilhado entre abas e gravado em
  disco sem ajuste.

## 7. Valores e objetos na memória

**Valor (32 bits)**:

```
 ...xxxxxxx0   inteiro de 31 bits (o bit 0 = 0)
 ...xxxxxxx1   referência: deslocamento dentro da gaiola da página (o bit 0 = 1)
```

`undefined`, `null`, `true`, `false` e a string vazia são objetos fixos no início da
gaiola. Um `double` vai numa **caixa de número**, ou direto no campo quando a forma diz
que aquele campo é sempre `double`.

**Objeto**:

```
 [forma 32b][extras 32b][elementos 32b][campo 0][campo 1]…   (campos "na linha")
```

- `forma`: aponta para a forma oculta (seção 8).
- `extras`: propriedades que não couberam na linha.
- `elementos`: itens indexados (`[0]`, `[1]`…), com o **tipo de elementos** na forma:
  inteiros, `double`, valores, e as versões com buracos.

Os dois objetos do exemplo, `{x:1, y:2}` e `{x:3, y:4}`, ocupam 20 bytes cada, sem
nenhuma tabela de nomes: os nomes moram só na forma, que é a mesma para os dois.

## 8. Formas ocultas e caches inline: o perfil

**Formas**: objetos criados do mesmo jeito compartilham uma forma. Acrescentar uma
propriedade segue uma transição:

```
F0 (Object.prototype, sem campos)
 └─ +x → F1 { x: campo 0 }
          └─ +y → F2 { x: campo 0, y: campo 1 }
```

Cada forma tem, além do endereço, uma **identidade estável**: o hash do caminho a partir da
raiz (protótipo + nomes e atributos, em ordem). Duas sessões que montam o objeto do mesmo
jeito chegam ao mesmo hash. É isso que permite gravar perfil e código em disco.

**Caches inline**: cada cache de instrução guarda o que viu.

| Cache | Instrução | O que vê na 1ª execução | Estado |
|---|---|---|---|
| `c0` | `pontos.length` | array com elementos "valores" | monomórfico: comprimento de array |
| `c1` | `i < …` | inteiro com inteiro | só inteiros |
| `c2` | `pontos[i]` | array, índice inteiro dentro do limite | monomórfico: elemento de array |
| `c3` | `.x` | forma `F2`, campo 0 | monomórfico: `F2 → campo 0` |
| `c4` | `total + …` | inteiro + inteiro, sem estouro | só inteiros |
| `c5` | `i++` | inteiro | só inteiros |

Estados possíveis: vazio → monomórfico → polimórfico (até 4 formas) → megamórfico (uma
tabela global forma×nome). Cada cache é descrito numa **linguagem pequena de guardas e
ações**:

```
c3:  GUARDA_FORMA  obj, F2   →  LE_CAMPO  obj, 0
```

O N1 interpreta essa descrição; o N2 gera código de máquina a partir dela; o N3 a usa como
aposta. É **uma** descrição para os três níveis (seção 2, A1, do `PLANO-JIT.md`).

## 9. N1: o intérprete

**Quadro de pilha** (o mesmo no N1 e no N2):

```
 [retorno][quadro anterior][função][contexto][acc][r0][r1]…[a0][a1]…
```

- **Primeira versão em Rust**: um laço `loop { match instrução { … } }`. É portátil e fica
  para sempre como reserva em plataforma que não permite gerar código.
- **Segunda versão gerada pelo nosso montador** na inicialização: cada instrução vira um
  trecho de máquina que termina saltando direto para o tratador da próxima. Usa o mesmo
  quadro do N2, então trocar de nível no meio de um laço é só pular para outro endereço.
- **Calor**: cada chamada soma no contador da função; cada `SALTA_ATRAS` soma no contador do
  laço. Passando do limite, a função é marcada para o N2 e, depois, para o N3. Os limites
  são ajustados pela bateria de sites (e sobem na bateria do notebook e em aba escondida).

## 10. N2: JIT base

Uma passada sobre o bytecode: cada instrução vira um **modelo** de código de máquina, com o
cache embutido. `LE_PROP_ACC k1, c3` (o `.x`), em x86-64, fica assim (esquema):

```
    test  eax, 1                 ; acc é referência?
    jz    lento                  ; não: caminho genérico
    mov   ecx, [gaiola + rax - 1]        ; forma do objeto
    cmp   ecx, [perfil.c3.forma]         ; é a forma que o cache viu?
    jne   lento
    mov   edx, [perfil.c3.campo]
    mov   eax, [gaiola + rax - 1 + 12 + rdx*4]   ; lê o campo
    …
lento:
    call  cache_c3_lento         ; atualiza o cache (pode virar polimórfico)
```

- Sem otimização entre instruções. O ganho vem de tirar a decodificação e o despacho do
  intérprete, e de o cache ficar na linha.
- Compila em microssegundos, na própria thread da página.
- Como o quadro é o mesmo do N1, a troca de nível pode acontecer em qualquer `SALTA_ATRAS`
  (troca no meio do laço, OSR).

## 11. N3: JIT otimizador

**1. Grafo SSA**, construído do bytecode e dos caches. Cada aposta vira uma **guarda**
explícita:

```
v0  = parâmetro pontos
      GUARDA_ARRAY_VALORES v0                          → desotimiza #1
v1  = COMPRIMENTO v0
laço (v2 = i: 0, v3 = total: 0):
   v4  = MENOR_INT v2, v1   ; se falso, sai do laço
   v5  = ELEMENTO v0, v2                               (limite: provado, ver passo 2)
         GUARDA_FORMA v5, F2                           → desotimiza #2
   v6  = CAMPO v5, 0
         GUARDA_INT v6                                 → desotimiza #3
   v7  = SOMA_INT_CHECADA v3, v6                       → desotimiza #4 (estouro)
   v8  = SOMA_INT v2, 1
retorna v3
```

**2. Otimizações**, na ordem de ganho:

- **Inlining** de funções pequenas e de builtins (`push`, `Math.*`, `charCodeAt`…), e das
  leituras do DOM declaradas como "lê o DOM" (`PLANO-JIT.md`, A3).
- **Guardas repetidas** saem: a mesma forma conferida duas vezes vira uma conferência.
- **Laço**: `COMPRIMENTO v0` sai de dentro do laço, porque nada no laço escreve em `pontos`.
- **Checagem de limite** de `ELEMENTO v0, v2`: sai, porque `v2 < v1` e `v1 = comprimento`.
  O passo anota essa **prova**, e um verificador independente a refaz antes de instalar o
  código. Se a prova não fecha, a checagem fica (`PLANO-JIT.md`, A4).
- **Objetos que não escapam** viram registradores (sem alocação).
- **Alocação de registradores** por varredura linear.

**3. Código final** do laço: algumas instruções por volta, com a forma conferida e a soma
checada contra estouro. Os registradores ficam na CPU, sem tocar o quadro.

**4. Onde roda**: numa **thread auxiliar**. Ela recebe o bytecode e uma cópia do perfil (os
dois imutáveis) e devolve o código pronto. A thread da página instala o código no próximo
ponto seguro (chamada ou `SALTA_ATRAS`) e nunca espera o otimizador.

## 12. Desotimização: quando a aposta falha

Cada guarda carrega o **estado do quadro** naquele ponto: em que registrador da máquina
está cada registrador do N1 (`r0` = total, `r1` = i, `acc`…) e em que instrução do
bytecode retomar.

Se chega um ponto `{x: 1.5}` (o `x` não é inteiro), a guarda #3 falha:

1. monta o quadro do N1 a partir do estado anotado;
2. continua no intérprete, na instrução `SOMA` (deslocamento 24);
3. anota no perfil: "`c4` viu `double`".

Mais tarde a função volta ao N3 com a aposta nova (soma de `double`). Função que
desotimiza demais fica no N2. O F12 mostra cada desotimização com o motivo.

## 13. Coletor de lixo e o código compilado

- **Pontos seguros**: o coletor só roda em chamadas e voltas de laço. Nesses pontos, o N2
  e o N3 têm um **mapa de pilha** que diz quais registradores e espaços do quadro guardam
  referências.
- **Barreira de escrita**: toda escrita de referência num objeto velho anota o objeto, para
  a coleta do berçário não varrer o heap inteiro. O N3 tira a barreira quando prova que o
  valor escrito é inteiro.
- **Objetos que se movem**: como os valores são deslocamentos dentro da gaiola, mover um
  objeto só exige atualizar os deslocamentos que apontam para ele, e o mapa de pilha diz
  onde estão.
- **Feito (26/10/2026, `coletor.rs`)**: menor copia os vivos do berçário para a velha
  (Cheney, com os lembrados); maior marca e compacta deslizando (endereço novo pelo mapa de
  bits, sem tabela por objeto) quando a velha dobra; WeakMap/WeakSet efêmeros.
  `FALCAO_GC_ESTRESSE=1` coleta em todo ponto seguro.

## 14. Exceções, geradores e async

- **Exceção**: lançar procura na tabela de exceções da função atual. Se não acha, desfaz o
  quadro e procura na anterior. No N2/N3, o mesmo caminho, pelo mapa de bytecode.
- **Gerador e `async`**: o quadro é copiado para um objeto no heap no `yield`/`await` e volta
  para a pilha no `next()`/retomada. O bytecode tem uma instrução de retomada que salta para
  o ponto certo.

## 15. Caches em disco (segunda visita)

Por **hash do script + versão do motor**:

| O que | Para que |
|---|---|
| tabela da pré-análise | pular a pré-análise |
| bytecode das funções que rodaram | pular léxico, sintaxe, escopos e gerador |
| perfil (formas pelo hash estável, tipos, desotimizações) | já saber o que vai esquentar |
| código N2/N3 relocável + lista de relocações | começar no nível alto |

Fica tudo na pasta de dados do navegador, só local, com teto de tamanho, e é apagado junto
com o cache.

## 16. Peças (crates) e como se ligam

| Crate | Etapas | Entrada → saída |
|---|---|---|
| `falcao-sintaxe` | 2, 3, 4, 5 | texto → símbolos → árvore → escopos (+ pré-análise) |
| `falcao-bytecode` | 6 | árvore + escopos → `FuncaoBytecode` (código, constantes, nº de registradores e de caches, tabela de exceções, mapa de posições); verificador do formato |
| `falcao-runtime` | 7, 8, 9, 13, 14 | valores, gaiola, formas, objetos, strings, builtins, coletor, caches inline, intérprete N1 em Rust |
| `falcao-asm` | — | montador x86-64 e AArch64 |
| `falcao-jit` | 9 (gerado), 10, 11, 12 | intérprete gerado, N2, N3, verificador de provas, desotimização, OSR |
| `falcao-regex` | — | expressões regulares |
| `falcao` | 15 | API de embutir (usada pelo `featherweb-js`), caches em disco |

As setas só descem: a sintaxe não conhece o runtime, o runtime não conhece o JIT (ele só
oferece os ganchos: contadores, caches, quadro). O `unsafe` fica só em `falcao-asm` e
`falcao-jit` (e no acesso cru à gaiola, isolado num módulo do runtime).

## 17. Ordem de construção

**Regra (pedido do usuário, 26/10/2026): o Falcão nasce no formato definitivo.** Nenhuma
etapa é um protótipo para refazer depois: o primeiro intérprete já usa os valores de 32
bits, a gaiola, as formas ocultas, os caches inline (na linguagem de guardas da seção 8) e
o quadro de pilha da seção 9, que é o mesmo do N2. O que chega depois (N2, N3, caches em
disco) só acrescenta, não troca o que já existe.

1. Sintaxe completa (léxico, árvore em arena, escopos, pré-análise), testada sozinha.
2. Runtime no formato final: gaiola, valores de 32 bits, formas com identidade estável,
   objetos com campos na linha e tipos de elementos, strings, coletor com berçário e
   barreira de escrita (mesmo que no começo só o berçário seja coletado em separado).
3. Bytecode final (com índices de cache e contadores de calor) + intérprete N1 em Rust
   sobre esse runtime, já com os caches inline monomórficos/polimórficos.
4. Builtins, exceções, Promises, geradores/async, módulos. **Primeiro caso de medida: o
   worker do reCAPTCHA** (só JS, mensagens e temporizadores, sem DOM), que no `boa` leva
   ~7 s e no Firefox menos de 0,4 s.
5. Troca do `boa` pela tabela única de ligações no `featherweb-js` (página com DOM).
6. Montador x86-64, intérprete gerado e N2.
7. N3, desotimização, verificador de provas.
8. Caches de perfil e de código em disco; AArch64; regex própria; WebAssembly.

Correspondência com as fases do `PLANO-JIT.md`: os itens 1 a 5 cobrem o J1 **e** o J2
juntos (formas, caches e valores de 32 bits deixam de ser uma fase à parte), o 6 é o J3,
o 7 é o J4, e o 8 cobre do J5 ao J8.
