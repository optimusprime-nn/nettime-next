"""Build-time packaging only; the native executables do not invoke Python."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parent

def run(*args, capture=False):
    return subprocess.run(list(map(str,args)), cwd=ROOT, check=True, text=True,
                          capture_output=capture).stdout

def main():
    parser=argparse.ArgumentParser()
    parser.add_argument('--iscc',type=Path,required=True)
    parser.add_argument('--cargo',default='cargo')
    args=parser.parse_args()
    cargo=args.cargo
    run(cargo,'fmt','--check')
    run(cargo,'test','--locked')
    run(cargo,'build','--release','--locked')
    from check_imports import verify
    for name in ('NetTimeNextNative.exe','NetTimeNextNativeService.exe'):
        verify(ROOT/'target'/'release'/name)
    metadata=json.loads(run(cargo,'metadata','--locked','--format-version','1',capture=True))
    notices=ROOT/'target'/'third-party-licenses'
    notices.mkdir(parents=True,exist_ok=True)
    for old in notices.iterdir():
        if old.is_file():
            old.unlink()
    packages=[]
    for package in metadata['packages']:
        if package['source'] is None:
            continue
        folder=Path(package['manifest_path']).parent
        licenses=[]
        candidates=list(folder.glob('*'))
        if package.get('license_file'):
            candidates.append(folder/package['license_file'])
        for file in candidates:
            if file.is_file() and (file.name.upper().startswith(('LICENSE','LICENCE','COPYING','NOTICE','COPYRIGHT')) or file==folder/(package.get('license_file') or '<none>')):
                target=package['name']+'-'+package['version']+'-'+file.name
                shutil.copy2(file,notices/target)
                licenses.append(target)
        packages.append(dict(name=package['name'],version=package['version'],license=package['license'],repository=package['repository'],license_files=licenses))
    (notices/'DEPENDENCIES.json').write_text(json.dumps(packages,indent=2)+'\n',encoding='utf-8')
    run(args.iscc.resolve(),ROOT/'installer.iss')
    installer=ROOT.parent/'dist'/'NetTimeNext-Rust-0.4.2-Preview-Setup-x64.exe'
    manifest=dict(version='0.4.2-preview',stage='Redesigned native dashboard, grouped settings, readable clock evidence and source reports; observe by default',installer=installer.name,
                  sha256=hashlib.sha256(installer.read_bytes()).hexdigest(),
                  source_commit=run('git','rev-parse','HEAD',capture=True).strip(),
                  source_dirty=bool(run('git','status','--porcelain','--untracked-files=no',capture=True)),
                  rustc=run(Path(cargo).with_name('rustc.exe') if Path(cargo).is_absolute() else 'rustc','--version',capture=True).strip(),
                  crt_linkage='static',runtime_imports_check='passed',signed=False)
    (installer.parent/'rust-build-manifest.json').write_text(json.dumps(manifest,indent=2)+'\n',encoding='utf-8')
    (installer.parent/'RUST-SHA256SUMS.txt').write_text(f"{manifest['sha256']}  {installer.name}\n",encoding='utf-8')
    print(json.dumps(manifest,indent=2))

if __name__=='__main__':
    main()
