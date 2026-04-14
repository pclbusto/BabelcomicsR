#!/usr/bin/env python3
"""
Migra los thumbnails de comics de la estructura plana:
  comics/{size}/{id}.jpg

a la estructura sharded:
  comics/{size}/{bucket}/{id}.jpg

donde bucket = (id // 1000) * 1000

Uso:
  python3 migrate_thumbnails.py [ruta_thumbnails]

Si no se pasa ruta usa ~/.local/share/babelcomics/thumbnails
"""

import os
import sys
import shutil

SHARD_SIZE = 1000
SIZES = ["small", "medium", "large"]


def migrate(base: str) -> None:
    total_moved = 0
    total_skipped = 0
    total_errors = 0

    for size in SIZES:
        size_dir = os.path.join(base, "comics", size)
        if not os.path.isdir(size_dir):
            print(f"  [skip] {size_dir} no existe")
            continue

        # Solo archivos .jpg en la raíz de size_dir (los ya sharded están en subdirectorios)
        flat_files = [
            f for f in os.listdir(size_dir)
            if f.endswith(".jpg") and os.path.isfile(os.path.join(size_dir, f))
        ]

        if not flat_files:
            print(f"  [ok]   {size_dir} ya está migrado o vacío")
            continue

        print(f"  Migrando {len(flat_files)} archivos en {size_dir} ...")

        for filename in flat_files:
            stem = filename[:-4]  # quitar .jpg
            try:
                comic_id = int(stem)
            except ValueError:
                print(f"    [warn] nombre inesperado, se ignora: {filename}")
                total_skipped += 1
                continue

            bucket = (comic_id // SHARD_SIZE) * SHARD_SIZE
            bucket_dir = os.path.join(size_dir, str(bucket))
            os.makedirs(bucket_dir, exist_ok=True)

            src = os.path.join(size_dir, filename)
            dst = os.path.join(bucket_dir, filename)

            if os.path.exists(dst):
                # Ya existe en destino, borrar el plano
                os.remove(src)
                total_skipped += 1
                continue

            try:
                shutil.move(src, dst)
                total_moved += 1
            except Exception as e:
                print(f"    [error] {filename}: {e}")
                total_errors += 1

    print(f"\nResultado: {total_moved} movidos, {total_skipped} omitidos, {total_errors} errores")


def main() -> None:
    if len(sys.argv) > 1:
        base = sys.argv[1]
    else:
        home = os.path.expanduser("~")
        base = os.path.join(home, ".local", "share", "babelcomics", "thumbnails")

    print(f"Directorio de thumbnails: {base}\n")

    if not os.path.isdir(base):
        print(f"Error: el directorio no existe: {base}")
        sys.exit(1)

    migrate(base)


if __name__ == "__main__":
    main()
