function f() { let s = 0; for (let i = 0; i < 3e7; i++) { s = (s + i) | 0; } return s; }
var r = f();
