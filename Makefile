# 本地检查入口：`make check` 跑完 CI 里会跑的所有项目
#
#   make check        # fmt + clippy + 单元测试 + viewer 测试 + 渲染基准
#   make check-fast   # 跳过渲染基准（无 GPU 环境或只想快速验证时）
#   make golden-update# 确认画面改动符合预期后，刷新渲染基准
#
# 渲染基准（tools/golden.sh）需要 assets/ 里有权重模型和可用 GL；缺条件时会自动 SKIP。

CARGO ?= cargo
export CARGO_HOME ?= $(CURDIR)/../.cargo-home

.PHONY: check check-fast fmt clippy test viewer golden golden-update build clean

check: fmt clippy test viewer golden

check-fast: fmt clippy test viewer

fmt:
	@echo "== cargo fmt --check =="
	@$(CARGO) fmt --check

clippy:
	@echo "== cargo clippy -D warnings =="
	@$(CARGO) clippy --release --offline -- -D warnings

test:
	@echo "== cargo test =="
	@$(CARGO) test --release --offline

viewer:
	@echo "== viewer 布局测试 =="
	@node src/viewer.test.cjs

golden:
	@echo "== 渲染基准 =="
	@sh tools/golden.sh

golden-update:
	@sh tools/golden.sh --update

build:
	@$(CARGO) build --release --offline

clean:
	@$(CARGO) clean
