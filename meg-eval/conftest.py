import sys
from pathlib import Path

_EVAL_DIR = str(Path(__file__).resolve().parent)
if _EVAL_DIR not in sys.path:
    sys.path.insert(0, _EVAL_DIR)