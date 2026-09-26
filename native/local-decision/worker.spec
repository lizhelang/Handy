# PyInstaller spec for the optional Apple Silicon Laya worker.
from PyInstaller.utils.hooks import collect_data_files, collect_submodules

hidden = []
for package in ("laya_mlx", "mlx", "tokenizers"):
    hidden.extend(collect_submodules(package))

datas = collect_data_files("laya_mlx") + collect_data_files("mlx")

a = Analysis(
    ["worker.py"],
    pathex=["native/local-decision"],
    binaries=[],
    datas=datas,
    hiddenimports=hidden,
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[],
    noarchive=False,
)
pyz = PYZ(a.pure)
exe = EXE(pyz, a.scripts, a.binaries, a.datas, name="inputia-decision-worker", console=True)
coll = COLLECT(exe, a.binaries, a.datas, strip=False, upx=False, name="inputia-decision-worker")
