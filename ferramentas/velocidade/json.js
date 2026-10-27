var dados = []; for (let i = 0; i < 2e4; i++) dados.push({ id: i, nome: 'n' + i, tags: ['a', 'b'], pos: { x: i / 3, y: -i } });
var r = 0; for (let k = 0; k < 10; k++) { const s = JSON.stringify(dados); r += JSON.parse(s).length + s.length; }
