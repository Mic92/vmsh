# Run the evaluation

<!--
Due its special hardware requirments we provide ssh access to our evaluation
machines. Please contact the paper author email address to obtain ssh keys. The
machines will have the correct hardware and also software installed to run the
experiments. If you run into problems you can write join the IRC channel #rkt-io
on freenode fro a live chat (there is also a webchat version at
https://webchat.freenode.net/) or write an email for further questions.
-->


The first step is to get the source code for rkt-io:

```console
$ git clone https://github.com/Mic92/rkt-io
```

For convience we created an evaluation script (reproduce.py) that will first build rkt-io and than run all evaluation experiments from the paper. 

This script only depends on Python and Nix as referenced above. All other dependencies will be loaded through nix. If the script fails at any point it can be restarted and it will only not yet done builds or experiments. Each command it runs will be printed to during evaluation along with environment variable set.

To run the evaluation script use the following command:

```console
$ cd rkt-io
$ python reproduce.py 
```

After the build is finished, it will start evaluations and generate graphs for each afterwards. The graphs will be written to ./results.

The following figures are reproduced:

<!--
    Figure 1. Micro-benchmarks to showcase the performance of syscalls, storage and network stacks across different systems
        a) System call latency with sendto()
        b) Storage stack performance with fio
        c) Network stack performance with iPerf

    Figure 5. Micro-benchmarks to showcase the effectiveness of various design choices in rkt-io Effectiveness of the SMP design w/ fio with increasing number of threads
        a) Effectiveness of the SMP design w/ fio with increasing number of threads
        b) iPerf throughput w/ different optimizations
        c) Effectiveness of hardware-accelerated crypto routines

    Figure 7. The above plots compare the performance of four real-world applications (SQlite, Ngnix, Redis, and MySQL) while running atop native linux
        a) SQLite throughput w/ Speedtest (no security) and three secure systems: Scone, SGX-LKL and rkt-io
        b) Nginx latency w/ wrk
        c) Nginx throughput w/ wrk
        d) Redis throughput w/ YCSB (A)
        e) Redis latency w/ YCSB (A)
        f) MySQL OLTP throughput w/ sys-bench
-->
