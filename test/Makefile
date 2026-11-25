run: build
	./test.elf
build:test.elf
test.elf:test.o ../target/debug/libabi_plan.a
	cc test.o ../target/debug/libabi_plan.a -o test.elf
test.o:test.c
	cc -c test.c -o test.o
