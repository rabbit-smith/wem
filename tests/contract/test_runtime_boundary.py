"""Strict structural boundary for the importable runtime package."""

from __future__ import annotations

import ast
import unittest
from dataclasses import fields
from pathlib import Path

from wwise2013_wem.scheduling.model import FramePlan
from wwise2013_wem.analysis.preprocessing.windowing import WindowedFrame


ROOT = Path(__file__).resolve().parents[2]
PACKAGE = ROOT / "src" / "wwise2013_wem"
PUBLIC_CLI_MODULES = {"__main__", "cli"}
BANNED_RUNTIME_MODULES = {"_capture_log", "residue_trace"}
BANNED_MODULE_NAME_MARKERS = ("capture", "fixture", "research")


def _module_files() -> dict[str, Path]:
    files: dict[str, Path] = {}
    for path in sorted(PACKAGE.rglob("*.py")):
        relative = path.relative_to(PACKAGE).with_suffix("")
        parts = relative.parts
        if parts[-1] == "__init__":
            module = ".".join(parts[:-1]) or "__init__"
        else:
            module = ".".join(parts)
        files[module] = path
    return files


def _trees() -> dict[str, ast.Module]:
    return {
        module: ast.parse(path.read_text(encoding="utf-8"), filename=str(path))
        for module, path in _module_files().items()
    }


def _source_package(module: str, path: Path) -> list[str]:
    if path.name == "__init__.py":
        return [] if module == "__init__" else module.split(".")
    return module.split(".")[:-1]


def _qualified_import_from(
    source: str,
    path: Path,
    node: ast.ImportFrom,
    known: set[str],
) -> set[str]:
    if node.level:
        package = _source_package(source, path)
        ascent = node.level - 1
        if ascent > len(package):
            return set()
        base = package[: len(package) - ascent]
        module_parts = node.module.split(".") if node.module else []
        qualified = ".".join([*base, *module_parts])
    else:
        module = node.module or ""
        if module == "wwise2013_wem":
            qualified = ""
        elif module.startswith("wwise2013_wem."):
            qualified = module[len("wwise2013_wem.") :]
        else:
            return set()

    targets: set[str] = set()
    if node.module is not None and qualified in known:
        targets.add(qualified)
    for alias in node.names:
        if alias.name == "*":
            continue
        candidate = ".".join(part for part in (qualified, alias.name) if part)
        if candidate in known:
            targets.add(candidate)
    return targets


def _qualified_plain_imports(node: ast.Import, known: set[str]) -> set[str]:
    targets: set[str] = set()
    prefix = "wwise2013_wem."
    for alias in node.names:
        if alias.name == "wwise2013_wem":
            targets.add("__init__")
        elif alias.name.startswith(prefix):
            candidate = alias.name[len(prefix) :]
            if candidate in known:
                targets.add(candidate)
    return targets


def _internal_import_graph() -> dict[str, set[str]]:
    files = _module_files()
    known = set(files)
    graph = {module: set() for module in known}
    for module, tree in _trees().items():
        candidates: set[str] = set()
        for node in ast.walk(tree):
            if isinstance(node, ast.Import):
                candidates.update(_qualified_plain_imports(node, known))
            elif isinstance(node, ast.ImportFrom):
                candidates.update(
                    _qualified_import_from(module, files[module], node, known)
                )
        graph[module].update(candidates)
    return graph


def _first_failure(title: str, failures: list[str]) -> str:
    ordered = sorted(failures)
    return title + "; first violation: " + ordered[0] + "\n" + "\n".join(ordered)


class RuntimeBoundaryTests(unittest.TestCase):
    def test_runtime_import_graph_is_acyclic(self):
        graph = _internal_import_graph()
        visited: set[str] = set()
        active: list[str] = []
        positions: dict[str, int] = {}
        cycles: list[str] = []

        def visit(module: str) -> None:
            visited.add(module)
            positions[module] = len(active)
            active.append(module)
            for target in sorted(graph[module]):
                if target not in visited:
                    visit(target)
                elif target in positions:
                    cycle = active[positions[target] :] + [target]
                    cycles.append(" -> ".join(cycle))
            active.pop()
            positions.pop(module)

        for module in sorted(graph):
            if module not in visited:
                visit(module)
        self.assertEqual(cycles, [])

    def test_recursive_names_and_relative_levels_are_fully_qualified(self):
        files = _module_files()
        self.assertIn("container.riff", files)
        self.assertIn("vorbis.codebook", files)
        self.assertNotIn("riff", files)
        self.assertNotIn("codebook", files)

        known = set(files)
        same_package = ast.parse("from .packets import extract_packets").body[0]
        parent_package = ast.parse("from .. import model").body[0]
        self.assertIsInstance(same_package, ast.ImportFrom)
        self.assertIsInstance(parent_package, ast.ImportFrom)
        self.assertEqual(
            _qualified_import_from(
                "container.wem", files["container.wem"], same_package, known
            ),
            {"container.packets"},
        )
        self.assertEqual(
            _qualified_import_from(
                "container.wem", files["container.wem"], parent_package, known
            ),
            {"model"},
        )

    def test_psychoacoustic_domains_have_canonical_owners(self):
        trees = _trees()
        expected_functions = {
            "analysis.config": {"make_wwise_psy_look"},
            "analysis.psychoacoustics.remap": {"wwise_psy_curve_smooth", "build_psy_remap"},
            "analysis.psychoacoustics.seed": {"wwise_seed_floor", "update_frame_spectrum_peak"},
            "analysis.psychoacoustics.envelope": {"shape_floor_envelope"},
            "analysis.psychoacoustics.pipeline": {"analyze_long_frame", "analyze_short_frame"},
        }
        owners: dict[str, str] = {}
        for module, expected in expected_functions.items():
            defined = {
                node.name
                for node in trees[module].body
                if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
            }
            self.assertTrue(expected <= defined, module)
            for name in expected:
                self.assertNotIn(name, owners)
                owners[name] = module
        analysis_functions = {
            node.name
            for node in trees["analysis.psychoacoustics.pipeline"].body
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        }
        self.assertEqual(
            analysis_functions, {"analyze_long_frame", "analyze_short_frame"}
        )

    def test_floor_domains_have_canonical_owners(self):
        trees = _trees()
        topology_and_codec = {
            "postlist_from_floor",
            "floor1_neighbor_tables",
            "floor1_wrap",
            "floor1_unwrap",
            "floor1_curve_from_posts",
        }
        fitting = {
            "floor1_fit_wwise",
            "floor1_fit_simple",
            "floor1_quantize_posts",
        }
        floor_names = {
            node.name
            for node in trees["vorbis.floor"].body
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        }
        fit_names = {
            node.name
            for node in trees["vorbis.floor_fit"].body
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef))
        }
        self.assertTrue(topology_and_codec <= floor_names)
        self.assertFalse(fitting & floor_names)
        self.assertTrue(fitting <= fit_names)
        self.assertFalse(topology_and_codec & fit_names)

    def test_analysis_session_does_not_depend_on_packet_or_container_layers(self):
        targets = _internal_import_graph()["analysis.session"]
        failures = [
            f"analysis.session: analysis imports downstream layer {target}"
            for target in sorted(targets)
            if target == "vorbis" or target.startswith("vorbis.")
        ]
        failures.extend(
            f"analysis.session: analysis imports container layer {target}"
            for target in sorted(targets)
            if target == "container" or target.startswith("container.")
        )
        self.assertFalse(
            failures,
            _first_failure("analysis dependency points downstream", failures)
            if failures
            else "",
        )

    def test_transient_detector_does_not_own_selector_policy(self):
        targets = _internal_import_graph()["analysis.transient.detector"]
        self.assertNotIn("scheduling.selector", targets)
        self.assertNotIn("analysis.session", targets)

    def test_mdct_and_transient_analysis_are_resource_io_free(self):
        graph = _internal_import_graph()
        for module in ("analysis.dsp.transform", "analysis.transient.detector"):
            self.assertFalse(
                any(target == "profiles" or target.startswith("profiles.") for target in graph[module]),
                module,
            )
            source = _module_files()[module].read_text(encoding="utf-8")
            for marker in ("ResourceRef", "importlib.resources", "read_json(", "read_bytes("):
                self.assertNotIn(marker, source, f"{module}: {marker}")

    def test_transient_implementation_has_one_module_owner(self):
        trees = _trees()
        transient_names = {
            node.name
            for node in trees["analysis.transient.detector"].body
            if isinstance(node, (ast.ClassDef, ast.FunctionDef))
        }
        transform_names = {
            node.name
            for node in trees["analysis.dsp.transform"].body
            if isinstance(node, (ast.ClassDef, ast.FunctionDef))
        }
        expected = {
            "WwisePsyHistory",
            "_wwise_psy_band_update",
            "wwise_psy_mask",
        }
        self.assertTrue(expected <= transient_names)
        self.assertFalse(expected & transform_names)

    def test_pcm_feeders_do_not_depend_on_session_or_selector_state(self):
        graph = _internal_import_graph()
        forbidden = {
            "analysis.session",
            "scheduling.selector",
            "analysis.transient.detector",
        }
        for module in (
            "analysis.preprocessing.detector_input",
            "analysis.preprocessing.windowing",
        ):
            self.assertFalse(graph[module] & forbidden, module)
        self.assertEqual(graph["scheduling.model"], set())

    def test_scheduling_package_does_not_depend_on_downstream_domains(self):
        graph = _internal_import_graph()
        forbidden = (
            "analysis",
            "application",
            "container",
            "profiles",
            "vorbis",
        )
        failures = []
        for source, targets in graph.items():
            if source != "scheduling" and not source.startswith("scheduling."):
                continue
            for target in targets:
                if any(
                    target == domain or target.startswith(domain + ".")
                    for domain in forbidden
                ):
                    failures.append(
                        f"{source}: scheduling imports downstream domain {target}"
                    )
        self.assertFalse(
            failures,
            _first_failure("scheduling dependency points downstream", failures)
            if failures
            else "",
        )

    def test_psychoacoustics_package_does_not_depend_on_downstream_domains(self):
        graph = _internal_import_graph()
        forbidden = ("application", "container", "profiles", "vorbis")
        failures = []
        for source, targets in graph.items():
            if source != "analysis.psychoacoustics" and not source.startswith(
                "analysis.psychoacoustics."
            ):
                continue
            for target in targets:
                if any(
                    target == domain or target.startswith(domain + ".")
                    for domain in forbidden
                ):
                    failures.append(
                        f"{source}: psychoacoustics imports downstream domain {target}"
                    )
        self.assertFalse(
            failures,
            _first_failure("psychoacoustics dependency points downstream", failures)
            if failures
            else "",
        )

    def test_analysis_dsp_does_not_depend_on_downstream_domains(self):
        graph = _internal_import_graph()
        forbidden = ("application", "container", "vorbis")
        failures = []
        for source, targets in graph.items():
            if source != "analysis.dsp" and not source.startswith("analysis.dsp."):
                continue
            for target in targets:
                if any(
                    target == domain or target.startswith(domain + ".")
                    for domain in forbidden
                ):
                    failures.append(
                        f"{source}: analysis DSP imports downstream domain {target}"
                    )
        self.assertFalse(
            failures,
            _first_failure("analysis DSP dependency points downstream", failures)
            if failures
            else "",
        )

    def test_window_model_has_one_analysis_preprocessing_owner(self):
        self.assertEqual(
            tuple(field.name for field in fields(WindowedFrame)),
            ("plan", "center", "samples"),
        )
        frame = WindowedFrame(
            FramePlan(4, 0, 1, 0, 0, 2048, 3072, 576),
            1024,
            ((0.0,) * 2048,),
        )
        self.assertEqual(
            (frame.index, frame.previous, frame.current, frame.following),
            (4, 0, 1, 0),
        )

    def test_runtime_has_no_research_named_modules(self):
        modules = set(_module_files())
        failures = [
            f"{module}: runtime module name contains {marker!r}"
            for module in modules
            for marker in BANNED_MODULE_NAME_MARKERS
            if marker in module.lower()
        ]
        failures.extend(
            f"{module}: deleted diagnostic module still exists"
            for module in modules & BANNED_RUNTIME_MODULES
        )
        self.assertFalse(
            failures,
            _first_failure("research module entered runtime", failures)
            if failures
            else "",
        )

    def test_runtime_does_not_import_deleted_diagnostics(self):
        failures: list[str] = []
        for module, tree in _trees().items():
            for node in ast.walk(tree):
                targets: set[str] = set()
                if isinstance(node, ast.Import):
                    targets.update(
                        alias.name.split(".")[-1] for alias in node.names
                    )
                elif isinstance(node, ast.ImportFrom):
                    if node.module:
                        targets.add(node.module.split(".")[-1])
                    targets.update(alias.name.split(".")[-1] for alias in node.names)
                for target in targets & BANNED_RUNTIME_MODULES:
                    failures.append(f"{module}: imports deleted module {target}")
        self.assertFalse(
            failures,
            _first_failure("deleted diagnostic dependency returned", failures)
            if failures
            else "",
        )

    def test_runtime_has_no_top_level_research_entrypoints(self):
        failures: list[str] = []
        for module, tree in _trees().items():
            for node in tree.body:
                if not isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    continue
                name = node.name.lower()
                if name.lstrip("_") == "self_test":
                    failures.append(f"{module}: top-level {node.name}")
                elif "first_fixture" in name:
                    failures.append(f"{module}: top-level {node.name}")
                elif name.startswith("verify_") and "capture" in name:
                    failures.append(f"{module}: top-level {node.name}")
        self.assertFalse(
            failures,
            _first_failure("research entrypoint entered runtime", failures)
            if failures
            else "",
        )

    def test_only_public_cli_modules_have_cli_structure(self):
        failures: list[str] = []
        for module, tree in _trees().items():
            if module in PUBLIC_CLI_MODULES:
                continue
            for node in ast.walk(tree):
                if isinstance(node, ast.Import) and any(
                    alias.name == "argparse" for alias in node.names
                ):
                    failures.append(f"{module}: imports argparse")
                elif isinstance(node, ast.ImportFrom) and node.module == "argparse":
                    failures.append(f"{module}: imports from argparse")
            for node in tree.body:
                if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)) and node.name == "main":
                    failures.append(f"{module}: top-level main")
                elif isinstance(node, ast.If) and "__name__" in ast.unparse(node.test):
                    failures.append(f"{module}: top-level __name__ guard")
        self.assertFalse(
            failures,
            _first_failure("hidden CLI entered runtime", failures)
            if failures
            else "",
        )


if __name__ == "__main__":
    unittest.main()
