# Internal parallelism: what other libraries do, and what the sources actually require

Date: 2026-09-22. A survey of external practice and of the normative documents
behind it, written to answer one question about this library. Everything below
was read from the owning source — a specification, a vendor's own documentation,
or the library's own source at a named version — on 2026-09-22. Where a claim
could not be traced to such a source it is marked **not confirmed** rather than
softened. Where the sources disagree with each other, that is written down.

## 1. The question

The owner put it as two layers:

- **the caller's task concurrency** — how many encodes, jobs or requests the
  embedding application chooses to run at once. That layer is the caller's and
  nothing here changes it;
- **our internal concurrency** — the threads one `encode` call starts by itself,
  inside the long-frame analysis, behind a compile-time `parallel` feature that
  is on by default.

The questions about the second layer, in his words: *is it a problem for a
library to provide concurrency out of nowhere? Or are there compatibility
problems on some machines? Or users who do not want the extra CPU? How do
similar libraries handle this, and is there a design guideline or code of
conduct — we must be standard.*

So the deliverable is not a preference. It is what the sources say, including
where they contradict what this repository does.

## 2. What each ecosystem does

### 2.1 `rayon` — the dependency this repository actually uses

Read at the version the lockfile builds: the workspace requires `rayon` 1.10.0
(`crates/Cargo.toml`) and `crates/Cargo.lock` resolves **rayon 1.12.0** with
**rayon-core 1.13.0**. Quotes below are from those crate sources as vendored by
the build, and the docs.rs pages render the same text:
[`rayon` 1.12.0 crate docs](https://docs.rs/rayon/1.12.0/rayon/),
[`ThreadPoolBuilder`](https://docs.rs/rayon-core/1.13.0/rayon_core/struct.ThreadPoolBuilder.html),
[`ThreadPool`](https://docs.rs/rayon-core/1.13.0/rayon_core/struct.ThreadPool.html).

**The global pool, and who sizes it.** The FAQ is the plainest statement of the
intended shape ([rayon 1.12.0, `FAQ.md`](https://github.com/rayon-rs/rayon/blob/main/FAQ.md)):

> "By default, Rayon uses the same number of threads as the number of CPUs
> available. Note that on systems with hyperthreading enabled this equals the
> number of logical cores and not the physical ones.
>
> If you want to alter the number of threads spawned, you can set the
> environmental variable `RAYON_NUM_THREADS` to the desired number of threads
> […]"

(the FAQ offers `ThreadPoolBuilder::build_global` as the alternative in the same
sentence).

**Who is meant to read `RAYON_NUM_THREADS` — the precise answer.** Rayon reads
it itself, when a pool is built without an explicit thread count. From the
resolver in rayon-core 1.13.0 (`src/lib.rs`, `get_num_threads`): if
`num_threads > 0` use it; otherwise consult `RAYON_NUM_THREADS`, then the
deprecated `RAYON_RS_NUM_CPUS`, then `std::thread::available_parallelism()`. The
public documentation of the setter states the same rule as a guarantee
([`ThreadPoolBuilder::num_threads`](https://docs.rs/rayon-core/1.13.0/rayon_core/struct.ThreadPoolBuilder.html#method.num_threads)):

> "If you specify a non-zero number of threads using this function, then the
> resulting thread pools are guaranteed to start at most this number of threads.
>
> If `num_threads` is 0, or you do not call this function, then the Rayon
> runtime will select the number of threads automatically. At present, this is
> based on the `RAYON_NUM_THREADS` environment variable (if set), or the number
> of logical CPUs (otherwise)."

Two consequences matter for this repository and both are documented, not
inferred:

1. `RAYON_NUM_THREADS` is read by rayon on behalf of **whoever runs the
   program**, and it reaches exactly those pools that were built without an
   explicit count. A pool built as
   `ThreadPoolBuilder::new().num_threads(n).build()` is documented to start *at
   most* `n` threads — the environment variable is not consulted for it.
2. Reconfiguring the global pool is a one-shot, process-wide act, and rayon
   discourages it
   ([`build_global`](https://docs.rs/rayon-core/1.13.0/rayon_core/struct.ThreadPoolBuilder.html#method.build_global)):
   "Calling `build_global` is not recommended, except in two scenarios: You wish
   to change the default configuration. You are running a benchmark […]" and
   "Initialization of the global thread pool happens exactly once. Once started,
   the configuration cannot be changed."

**Who reads `RAYON_NUM_THREADS`, in one line:** rayon does, when it constructs a
pool that was not given a count — the global pool at first use, or any custom
pool built without `num_threads` — and the FAQ addresses the person who runs the
program. No rayon documentation instructs library code to read it, and none says
a pool built with an explicit count should consult it; the resolver and
`num_threads` say the opposite.

On **per-call pools**, the documentation is silent in the sense that it never
recommends one: `ThreadPool` is an object a caller owns, `build_scoped` ties a
pool to a lexical scope, and the maintainer's answer on repeated work is that
"you don't need a custom pool for repeated use" — a statement about need, not a
prohibition.

**Does rayon tell libraries to use the global pool?** **No.** This must be
stated flatly, because it is the sentence one expects to find and it is not
there. Rayon 1.12.0's crate documentation, README, FAQ, `ThreadPoolBuilder`,
`ThreadPool`, `install` and `spawn` documentation contain no rule, recommendation
or example addressed to library authors about which pool to submit to. What the
crate docs do say is that private pools are a supported, first-class option
([`rayon` 1.12.0 README](https://github.com/rayon-rs/rayon)):

> "For even more control, you can create custom thread pools rather than using
> Rayon's default, global thread pool."

The maintainer's own statements, in the project's issue tracker, are the nearest
thing to guidance and they are permissive, not prescriptive. Asked when to use
`ThreadPoolBuilder`, Josh Stone (cuviper) answered in
[rayon#677](https://github.com/rayon-rs/rayon/issues/677):

> "The global thread pool remains for the life of the process. It can be
> manually initialized and configured with `ThreadPoolBuilder::build_global()`
> if you like, otherwise it will implicitly start with default settings on first
> use. A global 'use' would be any rayon invocation that's not explicitly
> installed in some other `ThreadPool` first.
>
> You would use separate `ThreadPool` instances if you need more control over
> the life of the threads, like `build_scoped()`, or if you want to segregate
> parallel tasks for any reason."

and on the cost of a private pool for repeated work:

> "The global initialization is only done once, so you don't need a custom pool
> for repeated use."

**What nesting is documented to do.** Rayon's functions run in *the pool you are
in*, not in a pool of their own
([`ThreadPool`](https://docs.rs/rayon-core/1.13.0/rayon_core/struct.ThreadPool.html)):

> "`install()` executes a closure in one of the `ThreadPool`'s threads. In
> addition, any other rayon operations called inside of `install()` will also
> execute in the context of the `ThreadPool`."

> "By contrast, top-level rayon functions (like `join()`) will execute
> implicitly within the current thread pool."

The maintainer restated it in [rayon#758](https://github.com/rayon-rs/rayon/issues/758):
"if you call this from within a pool already, it will continue on that pool,
otherwise it will use the global pool", and in
[rayon#909](https://github.com/rayon-rs/rayon/issues/909): "`rayon::scope` will
use whatever pool you're currently in, or else the global pool as a fallback."
A private pool inside a global-pool job does not deadlock — `install`'s
documented "Warning: execution order" says the calling thread "will try to keep
busy while the `op` completes in its target pool, similar to calling
`ThreadPool::yield_now()` in a loop" — but the two pools' threads both exist and
both compete for cores. Rayon also documents that the global pool is never torn
down (`spawn_handler` docs: "the global thread pool doesn't terminate until the
entire process exits!"), while a private pool is
("When the `ThreadPool` is dropped, that's a signal for the threads it manages to
terminate").

**The evidence that this matters to users** is a user's report, not a rule, in
[rayon#909](https://github.com/rayon-rs/rayon/issues/909):

> "I am using a library that is using rayon. I want to initialize the threadpool
> in a specific way (e.g. number of threads), and I want to do this as late as
> possible to incur overhead only when we need the threads. However, when I
> initialize it too late the threadpool might be initialized by code of a
> dependency."

That is the mechanism in one sentence: the application's lever is the global
pool, and a dependency that builds its own pool is outside that lever. Rayon
also restricts itself to one copy per binary for exactly this reason
(rayon-core docs, "Restricting multiple versions": "In order to ensure proper
coordination between thread pools, and especially to make sure there's only one
global thread pool, `rayon-core` is actively restricted from building multiple
versions of itself into a single target"), and the maintainer's answer in
[rayon#174](https://github.com/rayon-rs/rayon/issues/174) records the failure
mode of multiple pools: "those can't share threads […] but oversubscribed on
threads."

**Verdict for this section.** Rayon states no rule that a library must submit to
the global pool, so a private pool does not contradict a rayon *rule*. It does
contradict the shape rayon's documentation designs for — the global pool is the
one place an application can size, `RAYON_NUM_THREADS` is the documented way to
"alter the number of threads spawned", and a pool built with an explicit count
is documented not to consult it. Choosing a private pool therefore means the
application's documented lever does not reach our threads.

### 2.2 BLAS/LAPACK — the canonical libraries that take threads unasked

**OpenBLAS.** Its README has a section titled "Setting the number of threads
using environment variables" ([OpenBLAS `develop`
README](https://github.com/OpenMathLib/OpenBLAS/blob/develop/README.md)):

> "Environment variables are used to specify a maximum number of threads. For
> example,
>
> ```sh
> export OPENBLAS_NUM_THREADS=4
> export GOTO_NUM_THREADS=4
> export OMP_NUM_THREADS=4
> ```
>
> The priorities are `OPENBLAS_NUM_THREADS` > `GOTO_NUM_THREADS` >
> `OMP_NUM_THREADS`."

and for the runtime calls:

> "Note that these are only used once at library initialization, and are not
> available for fine-tuning thread numbers in individual BLAS calls."

The documented default is not in the README; the library's own source resolves
it ([`driver/others/init.c`](https://github.com/OpenMathLib/OpenBLAS/blob/develop/driver/others/init.c)):
after `OPENBLAS_NUM_THREADS`, `GOTO_NUM_THREADS`, `OMP_NUM_THREADS` and the
build-time `OPENBLAS_DEFAULT_NUM_THREADS` all come back empty,

> `if ((numprocs <= 0) || (numprocs > num_avail)) numprocs = num_avail;`

where `num_avail` is the count of CPUs in the process's affinity mask. So:
default-on, sized to the host, with an environment variable as the cap.

**Intel oneMKL.** Its developer guide (2026.0, document 766692, dated
2026-04-28, [oneMKL-specific environment variables for OpenMP threading
control](https://www.intel.com/content/www/us/en/docs/onemkl/developer-guide-windows/2026-0/onemkl-specific-env-vars-for-openmp-thread-ctrl.html))
lists `MKL_NUM_THREADS` as "Suggests the number of OpenMP threads to use", names
`OMP_NUM_THREADS` as its OpenMP equivalent, and states the design intent:

> "Use the Intel® oneAPI Math Kernel Library (oneMKL) -specific threading
> controls to distribute OpenMP threads between Intel® oneAPI Math Kernel
> Library (oneMKL) and the rest of your program."

The same guide's [Calling oneMKL Functions from Multi-threaded
Applications](https://www.intel.com/content/www/us/en/docs/onemkl/developer-guide-windows/2026-0/call-onemkl-functions-from-multi-threaded-apps.html)
documents the conflict as a named usage model:

> "Usage model: disable Intel® oneMKL internal threading for the whole
> application — When used: oneMKL internal threading interferes with
> application's own threading or may slow down the application. Example: the
> application is threaded at top level, or the application runs concurrently
> with other applications."

Two further documented properties bear on libraries generally, from the same
vendor's [Techniques to Set the Number of
Threads](https://www.intel.com/content/www/us/en/docs/onemkl/developer-guide-linux/2026-0/techniques-to-set-the-number-of-threads.html):

> "A call to the `mkl_set_num_threads` or `mkl_domain_set_num_threads` function
> changes the number of OpenMP threads available to all in-progress calls (in
> concurrent threads) and future calls to Intel® oneAPI Math Kernel Library
> (oneMKL) and may result in slow […] performance and/or race conditions".

> "You cannot change run-time behavior in the course of the run using the
> environment variables because they are read only once at the first call to
> Intel® oneAPI Math Kernel Library (oneMKL)."

**Apple Accelerate — not confirmed.** The `VECLIB_MAXIMUM_THREADS` variable that
is widely repeated for Accelerate could not be traced to an Apple-published page
in this environment: `developer.apple.com` served a bot check for the forum
thread that names it, and the Accelerate headers in the local SDK contain no
such symbol. Treat "Accelerate is threaded by default and capped by
`VECLIB_MAXIMUM_THREADS`" as **not confirmed from a first-party source** until
someone reads it off an Apple page.

**`threadpoolctl` — a tool whose reason to exist is this problem.** It is
maintained by the joblib project, whose repository description is itself the
statement ([joblib/threadpoolctl](https://github.com/joblib/threadpoolctl)):

> "Python helpers to limit the number of threads used in native libraries that
> handle their own internal threadpool (BLAS and OpenMP implementations)"

The README (master, version 3.7.0 released 2026-09-15; 3.8.0 in development)
gives the reason in its opening paragraph:

> "Fine control of the underlying thread-pool size can be useful in workloads
> that involve nested parallelism so as to mitigate oversubscription issues."

Its credits record where the code came from: "The initial dynamic library
introspection code was written by @anton-malakhov for the smp package available
at https://github.com/IntelPython/smp", and what it deliberately does not do:
"Contrary to smp, threadpoolctl does not attempt to limit the size of Python
multiprocessing pools (threads or processes) or set operating system-level CPU
affinity constraints: threadpoolctl only interacts with native libraries via
their public runtime APIs."

How strong is that evidence? It is strong *practice* evidence from the people
who own the affected workloads, and its own "Known Limitations" section
documents how hard the problem is to fix from outside:

> "Setting the maximum number of threads of the OpenMP and BLAS libraries has
> inconsistent scope and semantics (thread-local vs process-wide) depending on
> the underlying library."

Its "Semantics of thread limiting" section gives the concrete oversubscription
arithmetic for a library that keeps a pool per calling thread:

> "if you use MKL, each Python thread gets its own individual pool of worker
> threads from MKL, so if each Python threads calls a BLAS routine in MKL, you
> will get 10×10 = 100 MKL threads!"

The same failure is written up as a worked example by scikit-learn, which also
shows the mitigation it recommends ([scikit-learn user guide, "Parallelism",
fetched 2026-09-22](https://scikit-learn.org/stable/computing/parallelism.html)):

> "Oversubscription happens when a program is running too many threads at the
> same time. Suppose you have a machine with 8 CPUs. Consider a case where
> you're running a GridSearchCV (parallelized with joblib) with n_jobs=8 over a
> HistGradientBoostingClassifier (parallelized with OpenMP). Each instance of
> HistGradientBoostingClassifier will spawn 8 threads (since you have 8 CPUs).
> That's a total of 8 * 8 = 64 threads, which leads to oversubscription of
> threads for physical CPU resources and thus to scheduling overhead."

> "It is generally recommended to avoid using significantly more processes or
> threads than the number of CPUs on a machine."

Its BLAS section names the caps and their interaction: "`MKL_NUM_THREADS` sets
the number of threads MKL uses, `OPENBLAS_NUM_THREADS` sets the number of
threads OpenBLAS uses, `BLIS_NUM_THREADS` sets the number of threads BLIS uses.
Note that BLAS & LAPACK implementations can also be impacted by
`OMP_NUM_THREADS`."

So the BLAS family is: **default-on, host-sized, every implementation documents a
cap, and a whole tool exists because library-owned pools still escape the
caller's control in practice.** What the BLAS family does *not* show is a rule;
it shows an established, documented control surface and a measured failure mode.

### 2.3 OpenMP — the specification's rules for teams, nesting and limits

OpenMP is the only source here that is a specification, so its wording is
normative. The 5.2 HTML rendering at openmp.org has broken section links; the
complete HTML rendering is
[OpenMP 5.1 (November 2020)](https://www.openmp.org/spec-html/5.1/openmp.html)
and the current text is the OpenMP 6.0 PDF
([OpenMP API Specification 6.0, November 2024](https://www.openmp.org/wp-content/uploads/OpenMP-API-Specification-6-0.pdf)).
Quotes were checked against both the HTML rendering and the PDF of the same
version.

`OMP_NUM_THREADS` ([5.1 §6.2](https://www.openmp.org/spec-html/5.1/openmpse59.html),
6.0 §4.1.3):

> "The OMP_NUM_THREADS environment variable sets the number of threads to use
> for parallel regions by setting the initial value of the nthreads-var ICV."

and when it is unset the specification mandates no number — Table 2.1 (5.1) and
Table 3.2 (6.0), "ICV Initial Values", record `nthreads-var` as
"Implementation defined". *"It defaults to the number of cores"* is not spec
text anywhere.

`OMP_THREAD_LIMIT` ([5.1 §6.10](https://www.openmp.org/spec-html/5.1/openmpse67.html)):

> "The OMP_THREAD_LIMIT environment variable sets the maximum number of OpenMP
> threads to use in a contention group by setting the thread-limit-var ICV."

It bounds a contention group (5.1 glossary: "An initial thread and its descendent
threads"), not the program, and it is data-environment scoped. In 6.0 the wording
changes to "sets the number of threads to use for a contention group" and the
limit is split across `thread-limit-var`, `structured-thread-limit-var` and
`free-agent-thread-limit-var`.

**Nesting is off by default, and that is a rule.** 6.0 §1.2 (Execution Model):

> "parallel regions may be arbitrarily nested inside each other. If nested
> parallelism is disabled, or is not supported by the OpenMP implementation,
> then the new team that is formed by a thread that encounters a parallel
> construct inside a parallel region will consist only of the encountering
> thread."

The operative line is Algorithm 2.1 in 5.1 §2.6.1 (Algorithm 12.1 in 6.0):

> "else if ( active-levels-var ≥ max-active-levels-var ) then number of threads
> = 1;"

Nesting is enabled by the program, not by a library:
[`OMP_MAX_ACTIVE_LEVELS`](https://www.openmp.org/spec-html/5.1/openmpse65.html)
"controls the maximum number of nested active parallel regions by setting the
initial value of the max-active-levels-var ICV"; `OMP_NESTED` is deprecated in
5.0/5.1/5.2 and **absent from 6.0's normative text**, which says "All features
deprecated in versions 5.0, 5.1 and 5.2 were removed."

**Who owns the thread count.** The specification binds the count to the
*program's* control variables, and scopes a routine's effect to the calling
task's data environment:

> "An OpenMP implementation must act as if internal control variables (ICVs)
> control the behavior of an OpenMP program." (5.1 §2.4)

> "They are initialized by the implementation itself and may be given values
> through OpenMP environment variables and through calls to OpenMP API
> routines." (5.1 §2.4)

> "Calls to OpenMP API routines retrieve or modify data environment scoped ICVs
> in the data environment of their binding tasks." (5.1 §2.4.4)

> "The `omp_set_num_threads` routine affects the number of threads to be used
> for subsequent parallel regions that do not specify a num_threads clause, by
> setting the value of the first element of the nthreads-var ICV of the current
> task." ([5.1 §3.2.1](https://www.openmp.org/spec-html/5.1/openmpsu120.html))

and a formed team cannot be resized: "Once the team is created, the number of
threads in the team remains constant for the duration of that parallel region."
A nested team's budget is also shared, not multiplied — Algorithm 2.1 computes
`ThreadsAvailable = ( thread-limit-var - ThreadsBusy + 1)`, with `ThreadsBusy`
being the threads already executing in the contention group.

Two honest caveats: the specification contains **no sentence saying "this is the
program's decision and not a library's"** — that is an inference from the
ICV-ownership and data-environment rules above; and the word "oversubscription"
does not appear in this context in either version. The normative mechanism is
the `thread-limit-var` budget and the default serialisation of nested regions.

### 2.4 FFTW — opt-in, per plan, off by default

The FFTW manual (version 3.3.11; its own source comment dates the manual
2026-04-18, the project's release notes date 3.3.11 to 2026-04-21),
[Multi-threaded FFTW](http://www.fftw.org/fftw3_doc/Multi_002dthreaded-FFTW.html):

- the threads library is a **build-time opt-in**: "By default, the threads
  routines are not compiled" ([Installation on
  Unix](http://www.fftw.org/fftw3_doc/Installation-on-Unix.html));
- **initialization is an explicit call**:
  "Second, before calling any FFTW routines, you should call the function:
  `int fftw_init_threads(void);`";
- **the count is set by the caller, per plan creation**:

> "Third, before creating a plan that you want to parallelize, you should call:
> `void fftw_plan_with_nthreads(int nthreads);`"

- **the default is one thread**:

> "If you pass an nthreads argument of 1 (the default), threads are disabled for
> subsequent plans."

> "All plans subsequently created with any planner routine will use that many
> threads. You can call `fftw_plan_with_nthreads`, create some plans, call
> `fftw_plan_with_nthreads` again with a different argument, and create some
> more plans for a new number of threads. Plans already created before a call to
> `fftw_plan_with_nthreads` are unaffected."

- and the manual is explicit that the caller owes FFTW single-threaded planning
  ([Thread safety](http://www.fftw.org/fftw3_doc/Thread-safety.html)): "The
  upshot is that the only thread-safe routine in FFTW is `fftw_execute` (and the
  new-array variants thereof). All other routines (e.g. the planner) should only
  be called from one thread at a time."

FFTW therefore sits at the opposite end from BLAS: nothing happens unless the
caller asks, and the caller asks per plan.

### 2.5 Codec libraries — the closest analogue, and it is split

**libavcodec (FFmpeg).** The codec context field is documented as belonging to
the caller ([`avcodec.h`, doxygen trunk source, fetched
2026-09-22](https://ffmpeg.org/doxygen/trunk/avcodec_8h_source.html)):

> "thread count
> is used to decide how many independent tasks should be passed to execute()
> - encoding: Set by user.
> - decoding: Set by user."

and the codec option — the interface most callers use — documents the default
explicitly ([FFmpeg `doc/codecs.texi`](https://github.com/FFmpeg/FFmpeg/blob/master/doc/codecs.texi)):

> "@item threads @var{integer} (@emph{decoding/encoding,video})
> Set the number of threads to be used, in case the selected codec
> implementation supports multi-threading.
>
> Possible values:
> @table @samp
> @item auto, 0
> automatically select the number of threads to set
> @end table
>
> Default value is @samp{auto}."

So libavcodec's documented default is **auto-detect**, with an option to cap it.

**x264.** The public header defines the default
([x264.h](https://github.com/mirror/x264/blob/master/x264.h), the GitHub mirror
of the official tree; the project's own host was not reachable from this
environment, so treat the mirror as the copy read):

> `#define X264_THREADS_AUTO 0 /* Automatically select optimal number of threads */`

and `x264_param_default` sets it (`common/base.c`: `param->i_threads =
X264_THREADS_AUTO;`). The command-line help calls the option
"`--threads <integer>  Force a specific number of threads`". Default:
**auto-detect**, capped by an explicit option.

**libvpx.** The opposite documented default, in the same kind of header
([`vpx/vpx_encoder.h`](https://github.com/webmproject/libvpx/blob/main/vpx/vpx_encoder.h)):

> "Maximum number of threads to use
>
> For multi-threaded implementations, use no more than this number of threads.
> The codec may use fewer threads than allowed. The value 0 is equivalent to the
> value 1."

Default: **one thread**, threads only when the caller asks.

**libaom — not confirmed.** The `aom_codec_enc_cfg` structure carries a
`g_threads` field, but a primary copy of `aom_encoder.h` could not be fetched in
this environment (the project's own git host was unreachable and the GitHub
mirror no longer resolves), so its documented default is **not confirmed** here
and is not claimed.

**The honest summary of the family:** the mainstream default is **not**
uniformly single-threaded. libavcodec and x264 default to auto-detect; libvpx
defaults to single-threaded. Every one of them documents a caller-visible cap
(`threads`, `--threads`, `g_threads`). The survey's own brief assumed
single-threaded-with-opt-in was the mainstream default; at these versions it is
not — it is the libvpx and FFTW shape, while libavcodec, x264 and rayon are
auto-detect.

### 2.6 Rust ecosystem norms

**(a) `std::thread::available_parallelism` and containers.** Per its own
documentation ([std 1.98.1,
`available_parallelism`](https://doc.rust-lang.org/std/thread/fn.available_parallelism.html)),
it "Returns an estimate of the default amount of parallelism a program should
use", and the Linux notes say it does account for cgroup and affinity limits,
with named exceptions:

> "It may overcount the amount of parallelism available when limited by a
> process-wide affinity mask or cgroup quotas and sched_getaffinity() or cgroup
> fs can't be queried, e.g. due to sandboxing."

> "It does not attempt to take ulimit into account. If there is a limit set on
> the number of threads, available_parallelism cannot know how much of that
> limit a Rust program should take, or know in a reliable and race-free way how
> much of that limit is already taken."

> "It may overcount the amount of parallelism available when running in a VM
> with CPU usage limits (e.g. an overcommitted host)."

and it is not a hard limit: "The value returned by this function should be
considered a simplified approximation of the actual amount of parallelism
available at any given time."

So the owner's compatibility worry has a documented answer: on Linux, cgroup CPU
quotas are respected where they can be read; the documented failure cases are
unreadable cgroup/affinity state, `ulimit`, Windows affinity masks and job
objects, and CPU-limited VMs.

**(b) The library/executor rule.** The canonical Rust statement is about async
runtimes, in the official async book ([Async Rust, "The Async
Ecosystem"](https://rust-lang.github.io/async-book/08_ecosystem/00_chapter.html)):

> "Libraries exposing async APIs should not depend on a specific executor or
> reactor, unless they need to spawn tasks or define their own async I/O or
> timer futures. Ideally, only binaries should be responsible for scheduling and
> running tasks."

Tokio's own crate documentation states the library-side rule it enforces, in
terms of cost rather than prohibition ([tokio 1.53.1, `tokio/src/lib.rs`,
"Authoring libraries"](https://docs.rs/tokio/1.53.1/tokio/)):

> "As a library author your goal should be to provide the lightest weight crate
> that is based on Tokio. To achieve this you should ensure that you only enable
> the features you need. This allows users to pick up your crate without having
> to enable unnecessary features."

**There is no equivalent Rust statement about threads.** No Rust project's
documentation was found that says a library must not create a thread pool or a
worker thread for its caller. The general rule exists for *executors*; applying
it to threads is an argument by analogy, and this survey labels it as such.

**(c) Cargo features that change runtime behaviour.** The Cargo Book states the
additivity rule for features ([Cargo Reference, "Features"](https://doc.rust-lang.org/cargo/reference/features.html)):

> "A consequence of this is that features should be additive. That is, enabling
> a feature should not disable functionality, and it should usually be safe to
> enable any combination of features. A feature should not introduce a
> SemVer-incompatible change."

For behaviour that must be selectable, Cargo's own advice is a runtime option,
not a feature (same page, under "Mutually exclusive features"):

> "Architect the code to allow the features to be enabled concurrently, and use
> runtime options to control which is used. For example, use a config file,
> command-line argument, or environment variable to choose which behavior to
> enable."

The Rust API Guidelines carry exactly one rule about features, and it is about
naming, not behaviour ([checklist, C-FEATURE](https://rust-lang.github.io/api-guidelines/checklist.html):
"Feature names are free of placeholder words"). Nothing in the API Guidelines
addresses a feature that changes resource use at run time.

Read carefully, Cargo's rule does not forbid a thread-spawning feature: enabling
it does not disable functionality, and it is safe to combine. What Cargo does not
do is call it free — and Cargo's stated remedy for behaviour that a caller must
be able to switch is a runtime option. **The sources are silent on the exact
question "is a feature that spawns threads additive?"**; that judgement is not
made for us anywhere.

**(d) One-time global initialization is the application's job.** libcurl's
documentation for its own process-global initialization makes the ordering
constraint explicit
([`curl_global_init`](https://curl.se/libcurl/c/curl_global_init.html)):

> "If this is not thread-safe (the bit mentioned above is not set), you must not
> call this function when any other thread in the program (i.e. a thread sharing
> the same memory) is running."

That is a C library documenting that a *library* cannot safely perform its own
one-time global thread setup behind the caller's back; the caller has to be
given the chance to do it first.

### 2.7 Platform constraints that are documented

The question here is narrow and answerable: **on which targets can spawning
threads fail, and what are libraries documented to do about it.**

**WebAssembly: threads exist only if the embedder opted in.** The threads
proposal adds a memory type that is marked shared and some atomic operations,
and defers the rest
([WebAssembly threads proposal,
`Overview.md`](https://github.com/WebAssembly/threads/blob/main/proposals/threads/Overview.md)):

> "The responsibility of creating and joining threads is deferred to the
> embedder."

The proposal also makes an explicit maximum size a validation requirement for
that memory type. MDN's [`WebAssembly.Memory`
reference](https://developer.mozilla.org/en-US/docs/WebAssembly/Reference/JavaScript_interface/Memory/Memory)
states the constructor consequence ("Unshared WebAssembly memories don't need to
set a `maximum`, but shared memories do") and that a shared memory's buffer is a
`SharedArrayBuffer`. And a `SharedArrayBuffer` is only reachable in a
cross-origin isolated document
([MDN, `SharedArrayBuffer`](https://developer.mozilla.org/en-US/docs/Web/JavaScript/Reference/Global_Objects/SharedArrayBuffer)):

> "To use shared memory your document must be in a secure context and
> cross-origin isolated."

with the two headers named exactly
([MDN, `Window.crossOriginIsolated`](https://developer.mozilla.org/en-US/docs/Web/API/Window/crossOriginIsolated)):

> "A document will be cross-origin isolated if it is returned with an HTTP
> response that includes the headers: `Cross-Origin-Opener-Policy` header with
> the directive `same-origin`. `Cross-Origin-Embedder-Policy` header with the
> directive `require-corp` or `credentialless`."

So a Wasm build's ability to use threads is decided by whether the *page* is
served with those headers and by the embedder's shared memory — not by anything
a library can arrange for itself. Emscripten states the operational consequence
for its own pthreads support: "Pthreads code will not work in deployed
environment unless these headers are correctly set"
([Emscripten, Pthreads](https://emscripten.org/docs/porting/pthreads.html)).

**Rust states, as a platform rule, that `std::thread::spawn` panics there.** The
rustc book's page for `wasm32-unknown-unknown`
([Rust Platform Support](https://doc.rust-lang.org/rustc/platform-support/wasm32-unknown-unknown.html)):

> "The `wasm32-unknown-unknown` target has support for the Rust standard library
> but many parts of the standard library do not work and return errors. For
> example `println!` does nothing, `std::fs` always return errors, and
> `std::thread::spawn` will panic. There is no means by which this can be
> overridden."

**What a library is documented to do about it — rayon's two paths.** Rayon
distinguishes the pool it starts implicitly from a pool the caller asks for
([rayon 1.12.0, "Targets without
threading"](https://docs.rs/rayon/1.12.0/rayon/); [rayon-core 1.13.0, "Global
fallback when threading is
unsupported"](https://docs.rs/rayon-core/1.13.0/rayon_core/)):

> "The WebAssembly `wasm32-unknown-unknown` and `wasm32-wasi` targets are
> notable examples of this. Rather than panicking on the unsupported error when
> creating the implicit global thread pool, Rayon configures a fallback mode
> instead."

> "This fallback mode mostly functions as if it were using a single-threaded
> 'pool' […] Explicit `ThreadPoolBuilder` methods always report their error
> without any fallback."

That last sentence is the one that matters to a library that owns a pool: the
implicit global pool degrades, an explicit `ThreadPoolBuilder::build()`
**returns an error** and does not fall back. The failure is reportable, because
`std::thread::Builder::spawn` returns `Err(io::Error::UNSUPPORTED_PLATFORM)` on
those targets rather than panicking — the panic belongs to `std::thread::spawn`'s
own unwrap. A library that builds its own pool must therefore handle that error
itself, or not build a pool there at all.

**wasm-bindgen-rayon is the opt-in that makes threads exist on that target**, and
it fails at compile time when it is missing
([wasm-bindgen-rayon 1.3.0](https://docs.rs/wasm-bindgen-rayon/latest/wasm_bindgen_rayon/)):

> "WebAssembly thread support is not yet a first-class citizen in Rust - it's
> still only available in nightly"

> "the Rust standard library for the WebAssembly target is built without threads
> support to ensure maximum portability. Since we want standard library to be
> thread-safe and `std::sync` APIs to work, you'll need to use the nightly
> compiler toolchain and pass some flags to rebuild the standard library in
> addition to your own code."

with the documented build flags `-C target-feature=+atomics,+bulk-memory` and
`-Z build-std=panic_abort,std`, and a hard build error in its source when the
features are absent. Note what could **not** be confirmed: that page requires
cross-origin isolation ("In order to use `SharedArrayBuffer` on the Web, you need
to enable cross-origin isolation policies") but no first-party wasm-bindgen-rayon
sentence says the built module fails at run time without the headers; Emscripten
is the first-party source that states non-function.

**`no_std` and embedded: no thread API exists at all.** The `core` crate's own
documentation
([core 1.98.1](https://doc.rust-lang.org/core/)): "The core library is minimal :
it isn't even aware of heap allocation, nor does it provide concurrency or I/O."
The Embedded Rust Book puts the mechanism plainly
([A `no_std` Rust
environment](https://docs.rust-embedded.org/book/intro/no-std.html)): "`#![no_std]`
is a crate-level attribute that indicates that the crate will link to the
`core`-crate instead of the `std`-crate", and the standard library's runtime —
the part that includes "spawning the main thread" — "won't be available in a
`no_std` environment". A library that wants to be usable there cannot make
thread creation unconditionally reachable. (One correction to a common
formulation, since it decides how the portability argument is written:
`std::thread` is **not** conditional on a `std` feature in current Rust
source; the mechanism is `#![no_std]` dropping `std` from the crate graph
entirely.)

**iOS and Android: no documented hard limit, and both vendors document the cost
instead.** No first-party Apple or Google page stating a numeric per-process
thread cap was found, so any such number is **not confirmed**. What is
documented is per-thread cost and the recommendation to reuse pools. Apple's
archived Threading Programming Guide, "Thread Costs", gives a secondary thread
512 KB of stack (minimum 16 KB, multiple of 4 KB), about 1 KB of kernel data
structures and about 90 microseconds to create, and says of the alternatives:
"technologies such as GCD and operation objects are designed to manage threads
much more efficiently than your own code ever could by adjusting the number of
active threads based on the current system load"
([Apple Documentation Archive, 2014-07-15](https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/Multithreading/AboutThreads/AboutThreads.html)).
Android's threading guide says, of library-owned pools specifically
(["Better performance through threading", last updated
2026-05-19](https://developer.android.com/topic/performance/threads)):

> "Although from a software level, your code has the ability to create hundreds
> of threads, doing so can create performance issues. […] it's important to only
> create as many threads as your workload needs."

> "Each thread costs a minimum of 64k of memory. […] Many system processes and
> third-party libraries often spin up their own threadpools. If your app can
> reuse an existing threadpool, this reuse may help performance by reducing
> contention for memory and processing resources."

That is a platform vendor documenting the same norm from the receiving end:
libraries do spin up their own pools, and reuse is the preferred shape.

**The answer to the question.** Spawning can fail on `wasm32-unknown-unknown`
(the Rust platform page says `std::thread::spawn` panics; the builder path
returns `Unsupported`), on any target where the OS refuses a thread
(`ThreadPoolBuilder::build()` returns `ThreadPoolBuildError`; pthreads returns
`EAGAIN`), and it is impossible by construction on `no_std` targets. The
documented library responses are: degrade gracefully where the pool is implicit
(rayon's global fallback), report the error where the caller asked for a pool
(rayon's explicit builders), or fail the build when the target features are
missing (wasm-bindgen-rayon). **Treating "the host said no" as a reportable error
rather than a panic or an assumed success is what the sources support.**

### 2.8 Explicit design guidance

**C++ Core Guidelines.** The closest thing to a normative rule about libraries
and threads, in the concurrency section
([C++ Core Guidelines, fetched 2026-09-22](https://isocpp.github.io/CppCoreGuidelines/CppCoreGuidelines)):

> **CP.1: Assume that your code will run as part of a multi-threaded program**
> "It's hard to be certain that concurrency isn't used now or won't be used
> sometime in the future. Code gets reused. Libraries not using threads might be
> used from some other part of a program that does use threads. Note that this
> rule applies most urgently to library code and least urgently to stand-alone
> applications."

> **CP.41: Minimize thread creation and destruction** — "Reason: Thread creation
> is expensive." […] "Note: If your system has a good thread pool, use it. If
> your system has a good message queue, use it."

> **CP.4: Think in terms of tasks, rather than threads** — "A thread is an
> implementation concept, a way of thinking about the machine. A task is an
> application notion […] Application concepts are easier to reason about."

CP.1 is a rule for library authors and it points the other way from "we own the
threads": library code must assume the caller is already concurrent. CP.41 says
thread creation is expensive and to use an existing pool. Neither says a library
may not own a pool. The guidelines state no rule forbidding it — that has to be
recorded as silence, not as support.

**The rules that do exist, in one list.**

- The application owns the thread count, and the library submits work into a
  pool it did not size: **rule** in OpenMP (ICVs, data-environment scope,
  region-invariant team size); **rule-by-API-shape** in FFTW (the caller calls
  `fftw_plan_with_nthreads` per plan); **documented control surface** in BLAS
  (environment variables and setter functions) and in every codec surveyed.
- Internal parallelism must not multiply inside a caller that is already
  parallel: **rule** in OpenMP (nested region is one thread by default; the
  contention group's budget is shared).
- A caller must be able to cap or disable it without recompiling: **practice**,
  near-universal — `RAYON_NUM_THREADS`/`build_global`, `OPENBLAS_NUM_THREADS`/
  `MKL_NUM_THREADS`/`OMP_NUM_THREADS`, `fftw_plan_with_nthreads`,
  `thread_count`/`--threads`/`g_threads`, `threadpoolctl` for the libraries that
  made this hard.
- A library must not choose a process-global scheduling facility for its caller:
  **rule-shaped**, but only for async executors (async book); applying it to
  thread pools is an argument by analogy.
- Thread creation is expensive and can fail: **rule-shaped guidance** (CP.41) and
  **documented behaviour** (`ThreadPoolBuilder::build()` errors; rayon's wasm
  fallback covers only the implicit global pool).
- A Cargo feature must be additive and SemVer-compatible: **rule** (Cargo Book);
  the Cargo Book's advice for selectable behaviour is a runtime option.

## 3. The norms, distilled

Each row is a rule the sources actually support, with the strength of that
support. "Rule" means a specification, an official API document or a vendor
document states it as required behaviour; "practice" means the sources document
the same behaviour consistently but do not require it; "argument" means the
support is an analogy the survey is drawing explicitly.

| # | The norm | Support | Who states it |
| --- | --- | --- | --- |
| N1 | The caller decides how many threads its process runs; a library does not size the machine for it. | **Rule** (OpenMP); **API shape** (FFTW); **practice** (BLAS, codecs, rayon's `RAYON_NUM_THREADS`/`build_global`) | [OpenMP 5.1 §2.4, §3.2.1](https://www.openmp.org/spec-html/5.1/openmpsu120.html); [FFTW §5.2](http://www.fftw.org/fftw3_doc/Usage-of-Multi_002dthreaded-FFTW.html); [OpenBLAS README](https://github.com/OpenMathLib/OpenBLAS/blob/develop/README.md); [rayon FAQ](https://github.com/rayon-rs/rayon/blob/main/FAQ.md) |
| N2 | A library that uses threads must expose a documented cap or off switch the caller can set without recompiling. | **Rule** for OpenMP's environment variables; **practice** everywhere else | [OpenMP 5.1 §6.2, §6.10](https://www.openmp.org/spec-html/5.1/openmpse59.html); [oneMKL 2026.0](https://www.intel.com/content/www/us/en/docs/onemkl/developer-guide-windows/2026-0/onemkl-specific-env-vars-for-openmp-thread-ctrl.html); [FFmpeg `threads`](https://github.com/FFmpeg/FFmpeg/blob/master/doc/codecs.texi); [libvpx `g_threads`](https://github.com/webmproject/libvpx/blob/main/vpx/vpx_encoder.h) |
| N3 | Nested internal parallelism must not multiply the caller's threads; the platform's own mechanism serialises nesting by default. | **Rule** (OpenMP); **practice** (BLAS/`threadpoolctl`) | [OpenMP Algorithm 2.1](https://www.openmp.org/spec-html/5.1/openmpsu40.html); [threadpoolctl README](https://github.com/joblib/threadpoolctl) |
| N4 | A library must not reconfigure a process-global facility the caller may already have configured. | **Practice, strongly documented** — one-shot initialisation, vendor guidance to disable internal threading when the application is threaded | [rayon `build_global`](https://docs.rs/rayon-core/1.13.0/rayon_core/struct.ThreadPoolBuilder.html#method.build_global); [oneMKL multi-threaded usage models](https://www.intel.com/content/www/us/en/docs/onemkl/developer-guide-windows/2026-0/call-onemkl-functions-from-multi-threaded-apps.html); [`curl_global_init`](https://curl.se/libcurl/c/curl_global_init.html) |
| N5 | Thread creation is expensive, and a pool owned per call is worse than a pool reused. | **Rule-shaped guidance** (CP.41), **maintainer statement** (rayon#677) | [C++ Core Guidelines CP.41](https://isocpp.github.io/CppCoreGuidelines/CppCoreGuidelines); [rayon#677](https://github.com/rayon-rs/rayon/issues/677) |
| N6 | A library must not pick a runtime/executor for its caller; only the binary schedules. | **Rule, for async executors only** — for threads this is an **argument by analogy** | [Async Rust, The Async Ecosystem](https://rust-lang.github.io/async-book/08_ecosystem/00_chapter.html); [tokio 1.53.1, "Authoring libraries"](https://docs.rs/tokio/1.53.1/tokio/) |
| N7 | Threads may be unavailable or refused on some targets; the library needs a path that works there and must report refusal rather than assume success. | **Documented platform behaviour** (wasm needs shared memory plus isolation headers; `no_std` has no thread API); **documented runtime behaviour** (explicit pool build returns an error instead of falling back) | [rayon-core "Global fallback"](https://docs.rs/rayon-core/1.13.0/rayon_core/); [std `available_parallelism` errors](https://doc.rust-lang.org/std/thread/fn.available_parallelism.html) |
| N8 | A Cargo feature must be additive and SemVer-compatible; behaviour a caller must be able to select belongs in a runtime option. | **Rule** (additivity); **official advice** (runtime option) | [Cargo Reference, Features](https://doc.rust-lang.org/cargo/reference/features.html) |
| N9 | Whether *spawning threads when a feature is enabled* is "additive" is addressed nowhere. | **Silence** — the API Guidelines' only feature rule is about naming | [API Guidelines checklist, C-FEATURE](https://rust-lang.github.io/api-guidelines/checklist.html) |

Two disagreements between the sources are worth recording rather than
harmonising:

- **The default is not agreed.** FFTW and libvpx default to doing nothing;
  libavcodec, x264 and rayon default to using the machine. There is therefore no
  norm that says "default off"; there is a norm that says "documented and
  cappable".
- **The cap's semantics are not agreed.** threadpoolctl documents that "Setting
  the maximum number of threads of the OpenMP and BLAS libraries has
  inconsistent scope and semantics (thread-local vs process-wide) depending on
  the underlying library", and MKL documents that its global setter changes
  in-progress concurrent calls. A cap is not a clean concept across libraries;
  a cap that is *ours* — a knob on our own pool — is at least unambiguous.

## 4. Where this repository stands

The checkout moved during this survey: `main` now carries the pool-sizing change
(the commit that gives the channel waves a pool sized to the job count), so the
shape below was re-read from the merged tree rather than from the tree the
survey started on. The full evidence record for the change itself is
[`pool-sizing.md`](pool-sizing.md); the neighbouring axis — how many encodes to
run at once — is [`concurrency-curves.md`](concurrency-curves.md).

What the code does today:

- `crates/wem-analysis/src/psychoacoustics/pool.rs` builds
  `rayon::ThreadPoolBuilder::new().num_threads(workers).build()` with
  `workers = channels`, **once per analysis session**; the session owns it and
  drops it (`crates/wem-analysis/src/session.rs`); a build failure becomes
  `AnalysisError::PoolUnavailable`.
- The two channel waves spawn one job per channel into `pool.scope(...)` and
  collect by slot index, so results are in channel order at any pool size.
- `wem-core`'s `parallel` feature is `default = ["parallel"]`; `wem-analysis`
  declares it non-default and `wem-wasm` builds `wem-core` with
  `default-features = false`.
- No environment variable is read for sizing, and no global pool is configured.
  `channel_pool_workers()` reports the size; there is deliberately no setter.

Against the norms:

| Norm | Where this repository stands |
| --- | --- |
| N1 — the caller decides the process's thread count | **Conflicts, on the documented rayon mechanism, not on a rayon rule.** The pool is built with an explicit `num_threads`, and rayon documents that an explicit count is exactly the case where `RAYON_NUM_THREADS` is not consulted. An application that sets `RAYON_NUM_THREADS=1`, or that configures the global pool with `build_global`, cannot reach these workers: the lever rayon documents goes to pools built without a count. |
| N2 — a documented cap or off switch the caller can set without recompiling | **Conflicts in the shipped native build.** There is no runtime knob: not an environment variable (excluded by policy), not a builder option, not a public setter. The only switches are compile-time (`default-features = false`) or the pool's derived size. Every comparable library in section 2 exposes one. |
| N3 — internal parallelism must not multiply the caller's threads | **Matches, by measurement and by shape.** One worker per channel, and the same jobs run in whatever the caller's concurrency is; the repository's own two records put the wall optimum at the channel count and show the feature not paying for stereo at any concurrency (`concurrency-curves.md`, `pool-sizing.md`). |
| N4 — do not reconfigure a process-global facility | **Matches.** No `build_global`, no global state, two sessions do not affect each other. This is the norm the current shape satisfies most clearly, and it is the reason the change was made. |
| N5 — thread creation is expensive; reuse over per-call creation | **Partly.** The pool is per session, not per call, so a caller that encodes repeatedly in one session pays once. A caller that builds a session per encode pays thread start and teardown per encode — the cost CP.41 warns about, and the case rayon's maintainer described as unnecessary work when a global pool exists. |
| N6 — a library must not choose a runtime for its caller | **Silently outside the rule's scope, and analogous to a violation.** The async-book rule is about executors. The thread analogy — a library that decides how much CPU the caller's machine gives to one call — is the argument, not a citation, and it points against an unconditional private pool. |
| N7 — targets where threads are unavailable, and refusal reported not assumed | **Matches, and better than rayon's implicit pool would.** The feature is absent on `wasm32-unknown-unknown` through the `wem-wasm` shell, and where the pool is built, a host refusal is reported as `PoolUnavailable` rather than panicking. Worth recording precisely: rayon's *implicit global* pool would degrade gracefully on wasm, while an explicit `ThreadPoolBuilder::build()` — the shape now used — returns an error; this repository's wasm shell avoids the question by not compiling the feature at all. |
| N8 — features additive; selectable behaviour in a runtime option | **Partly, and the documented remedy differs from what we do.** `parallel` does not disable functionality and is safe to combine, so it satisfies Cargo's stated additivity rule; but its effect is a change in resource use at run time, and Cargo's stated remedy for behaviour a caller must be able to select is a config file, argument or environment variable — all three of which this repository deliberately does not have. |
| N9 — is a thread-spawning feature "additive"? | **The sources are silent.** No document surveyed answers it; the judgement is ours to make and to write down. |

Two further points of honesty about our own documents:

- `docs/reference/standards.md` ("Caller streams and ambient state") says "No
  environment variable, working directory, clock or locale decides what gets
  encoded". Read literally, that forbids ambient state from deciding *what* is
  encoded — and the pool size provably does not change a byte. The step from
  that sentence to "so the library may not read `RAYON_NUM_THREADS` to size a
  pool" is the repository's own extension of its norm, not the sentence's
  literal scope. Worth knowing, because an external reader will read the
  sentence the narrow way.
- `pool.rs`'s module comment says "A library may not fix that by reconfiguring
  the global pool." Rayon's documentation does not say that; it says
  `build_global` "is not recommended, except in two scenarios" and that
  initialisation happens once and cannot be changed. The conclusion is right;
  the citation behind it is a practice, not a rule, and the file should not read
  as if rayon forbade it.

Finally, the repository's own measurement is the strongest evidence in the
building and it is not external: on this machine, internal parallelism paid at
low concurrency and stopped paying at N between 3 and 6 depending on load, never
paid for stereo, and always cost more CPU per encode. That is a statement about
*when* the threads are worth having; it does not decide who may size them.

## 5. What the choice actually is

The options, with what each satisfies and what each breaks. No recommendation is
made here; the sources above are the input to the decision.

| Option | Satisfies | Breaks or risks | Documented consequence |
| --- | --- | --- | --- |
| **A. Submit to the current/global pool** (`rayon::scope` with no pool of our own), feature default-on | N1, N4, N5, N7 (graceful wasm fallback) | N2 (no cap of our own — but the caller's cap works, which is the point), N3 only by the caller's own sizing | `RAYON_NUM_THREADS` and `build_global` reach our jobs; the pool is 16 workers on a 16-core host against 6 jobs, which is the measured cost this repository just removed |
| **B. Private pool per call**, size = channel count | N3, N4 | N1, N2, N5 (thread start/teardown per call), N7 (explicit build errors on threadless targets) | Session-shaped pool is the shape in the tree; per-*call* pool construction is the CP.41 cost with no reuse |
| **C. Private pool per session**, size = channel count (**today's shape**), no knob | N3, N4, N5 (within a session), N7 | N1 (documented rayon lever no longer reaches it), N2 (no runtime cap) | One worker per channel; the caller cannot ask for fewer; the pool is a function of geometry, identical on every host |
| **D. C as above plus a caller-visible runtime knob** (builder option and/or an environment variable) | N1, N2, N3, N4, N5, N7 | The repository's stated position against reading ambient state; a knob is a new public surface | The caller can cap or disable; an environment variable is what every comparable library documents, and MKL's warnings about global setters show why a per-session option is the safer form |
| **E. `parallel` default-off** (opt-in, FFTW/libvpx shape) | N1, N2 in the strongest form the sources show | libavcodec, x264 and rayon are the counter-examples: auto-detect is mainstream in this domain | A caller who wants the throughput must find and enable a feature; the 1.18× at N = 1 on 6 channels (`concurrency-curves.md`) is lost by default |
| **F. No compile-time feature; decide at run time** (e.g. only parallelise above a channel or size threshold) | N2, N8 (behaviour in a runtime option, Cargo's stated remedy), keeps N3/N4 | A runtime decision on the library's side is still a decision; needs a documented rule for when it engages | Cargo's own advice for selectable behaviour is exactly this shape; threadpoolctl's existence shows callers want the decision to be theirs and legible |

## 6. What would falsify this repository's current shape

Named, specific statements that — if they exist as rules — would require changing
the private pool, the default, or the no-environment-reads position. Each entry
says where it would have to come from and what it would change.

1. **A rayon document requiring libraries to submit to the global pool** — e.g.
   a "Libraries should use the global pool" paragraph in the `rayon` or
   `rayon-core` crate docs, the FAQ or the `ThreadPool` documentation. *Not
   present in rayon 1.12.0 / rayon-core 1.13.0*: the docs are silent on library
   authorship, and the README explicitly offers custom pools. If a future rayon
   version states it, option C must go back to option A on the spot.
2. **A rayon document saying `RAYON_NUM_THREADS` is read for every pool** —
   including pools built with an explicit count. *Not present*: the opposite is
   documented, in `num_threads`'s "If you specify a non-zero number of
   threads…" and in the resolver's ordering. If it changed, the private pool
   would become caller-cappable by environment and the N1 conflict would
   disappear without a code change.
3. **A rule that a library must expose a runtime cap** — for OpenMP that rule
   exists but binds OpenMP implementations, not Rust libraries: `OMP_NUM_THREADS`
   and `OMP_THREAD_LIMIT` are the program's levers, and the specification's ICV
   rules are about implementations. Applying it here would require option D or
   F. The BLAS, FFTW and codec evidence is practice, not rule, and on its own
   does not compel a change — it does establish that callers expect a lever.
4. **A rule that the default must be single-threaded.** *No such rule exists.*
   FFTW and libvpx default to one thread; libavcodec, x264 and rayon default to
   auto-detect. The default is a project decision, and the only external
   constraint on it is that whatever the default is must be documented and
   cappable.
5. **Cargo's additivity rule read as covering resources.** The rule as written
   covers disabled functionality and SemVer compatibility, and `parallel`
   satisfies it. If the project reads it as also covering "enabling must not
   acquire machine resources", then per Cargo's own advice the switch belongs in
   a runtime option, and the feature should shrink to a pure compile-time
   availability flag — option F.
6. **A platform rule that threads are unavailable wherever we ship.** For
   `wasm32-unknown-unknown` without shared memory and the isolation headers,
   threads are unavailable, and the current shape (feature absent in the wasm
   shell) already respects it. A target where threads are unavailable but the
   feature *is* compiled in — a native `no_std` build, or a wasm build with the
   feature on and no isolation headers — would require the `PoolUnavailable`
   path to be the documented behaviour rather than an error at session
   construction.
7. **A measurement that flips the internal case.** `concurrency-curves.md`
   already falsifies the unconditional version of the current shape: internal
   parallelism never pays for stereo and stops paying at N between 3 and 6 under
   load. If a future record showed it paying at every geometry and concurrency,
   option A/E would lose their main empirical objection; if it showed it paying
   nowhere, option E would gain one.

## 7. What could not be confirmed

- **Apple Accelerate's threading controls.** No Apple-published page naming
  `VECLIB_MAXIMUM_THREADS` was reachable; the forum thread that states it is
  behind a bot check and is a forum reply, not documentation. Accelerate's
  default and its cap are therefore **not confirmed** here.
- **libaom's `g_threads` default.** No primary copy of `aom_encoder.h` was
  reachable from this environment; the field's documented meaning and default
  are **not confirmed**.
- **iOS and Android hard thread limits.** No first-party page stating a numeric
  per-process limit was found; the documented material describes per-thread cost
  and pool-reuse advice instead. Also not confirmed: any first-party Android
  statement tying a thread ceiling to a per-architecture virtual address space.
  The general "thread creation can be refused" behaviour is used instead.
- **wasm-bindgen-rayon failing at run time without the isolation headers.** Its
  documentation states the requirement but not the runtime failure; Emscripten is
  the first-party source that states non-function ("Pthreads code will not work
  in deployed environment unless these headers are correctly set").
- **A thread-specific version of the library/executor rule.** The canonical Rust
  statement is the async book's, about executors; the thread case is an argument
  by analogy and is labelled as one.
- **OpenMP's literal ownership sentence.** The specification binds thread counts
  to ICVs and the program that sets them; it does not contain a sentence saying
  "not a library's decision". That step is an inference from the ICV rules.
- **x264 through its own host.** The header and default were read from the
  GitHub mirror of the official repository because the project's own git host
  was unreachable; the values match the mirror's `master`.
- **"`std::thread` requires the `std` feature."** A common way to phrase the
  portability argument, and **contradicted** by the current Rust source: the
  module carries no such attribute. The accurate statement is the `#![no_std]`
  one used in section 2.7.