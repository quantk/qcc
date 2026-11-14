# Thin aliases for ./dev, which also works outside the nix shell (where make is absent).
.PHONY: run demo check test clean
run demo check test clean:
	./dev $@
