var a = []; for (let i = 0; i < 3e5; i++) a.push((i * 7919) % 1000003);
a.sort((x, y) => x - y); var b = a.map(String); b.sort();
