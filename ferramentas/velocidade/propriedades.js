function Ponto(x, y) { this.x = x; this.y = y; }
function f() { let s = 0; const ps = []; for (let i = 0; i < 1000; i++) ps.push(new Ponto(i, i * 2));
  for (let k = 0; k < 5000; k++) for (let i = 0; i < ps.length; i++) s = (s + ps[i].x + ps[i].y) | 0; return s; }
var r = f();
