function f() { let s = 0; for (let i = 0; i < 3e6; i++) { const o = { a: i, b: [i, i + 1], c: { d: i } }; s = (s + o.b[1] + o.c.d) | 0; } return s; }
var r = f();
