function cont() { let c = 0; return { inc: () => ++c, val: () => c }; }
var r = 0; for (let i = 0; i < 2e5; i++) { const k = cont(); for (let j = 0; j < 50; j++) k.inc(); r += k.val(); }
