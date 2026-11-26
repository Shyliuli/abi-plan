run: build
	./test.elf
build:test.elf
test.elf:test.o ../target/debug/libabi_plan.a
	cc -g test.o ../target/debug/libabi_plan.a -o test.elf
test.o:test.c
	cc -g -O0 -c test.c -o test.o
clean:
	rm ./test.elf ./test.o