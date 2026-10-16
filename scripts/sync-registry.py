#!/usr/bin/env python3
# Run from lints/.
# Regenerates the modules! list in lib.rs and the [[example]] tables in
# Cargo.toml from the files present in src/ and ui/.
import pathlib, re
root = pathlib.Path('.')
mods = sorted(p.stem for p in (root/'src').glob('*.rs') if p.stem not in ('lib', 'util', 'comment_scan'))
lib = (root/'src/lib.rs').read_text()
lib = re.sub(r'modules! \{\n.*?\n\}', 'modules! {\n' + ''.join(f'    {m},\n' for m in mods) + '}', lib, flags=re.S)
(root/'src/lib.rs').write_text(lib)
cargo = (root/'Cargo.toml').read_text()
cargo = re.sub(r'\n\[\[example\]\]\nname = "[^"]+"\npath = "[^"]+"\n(crate-type = [^\n]+\n)?', '\n', cargo)
cargo = re.sub(r'\n{3,}', '\n\n', cargo)
def example(u):
    lib = '#![crate_type = "lib"]' in u.read_text()
    return f'[[example]]\nname = "{u.stem}"\npath = "ui/{u.name}"\n' + ('crate-type = ["lib"]\n' if lib else '') + '\n'
examples = ''.join(example(u) for u in sorted((root/'ui').glob('*.rs')))
cargo = cargo.replace('[package.metadata.rust-analyzer]', examples + '[package.metadata.rust-analyzer]')
(root/'Cargo.toml').write_text(cargo)
print(mods)
