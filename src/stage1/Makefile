CARGO ?= cargo

obj-m += stage1.o

stage1-objs := module.o libstage1.o

# HACK: rename to .o as otherwise it seems to ignore it during linking
$(obj)/libstage1.o: $(RUST_DIR)/src/lib.rs
	(cd $(RUST_DIR) && $(CARGO) build --release)
	cp $(RUST_DIR)/target/release/libstage1.a $(obj)/libstage1.o
