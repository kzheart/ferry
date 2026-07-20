"""Historical Claude editing CLI and import compatibility entry point."""

from .adapters.claude.editing import *  # noqa: F401,F403
from .adapters.claude.editing import main


if __name__ == "__main__":
    main()
