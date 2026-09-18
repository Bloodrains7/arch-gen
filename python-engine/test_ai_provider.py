"""Offline tests: AI dependencies are mocked, no server or disk cache required."""

from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
from contextvars import ContextVar
import importlib
import os
import sys
from threading import Event
from types import SimpleNamespace
import unittest
from unittest.mock import MagicMock, patch

from ai_provider import AIProviderError, ProviderConfig, health_check, with_local_provider


class ProviderTests(unittest.TestCase):
    def setUp(self):
        self.environment = patch.dict(os.environ, {}, clear=True)
        self.environment.start()
        self.addCleanup(self.environment.stop)
        self.current_lm = ContextVar("test_lm", default=None)

        @contextmanager
        def context(*, lm):
            token = self.current_lm.set(lm)
            try:
                yield
            finally:
                self.current_lm.reset(token)

        self.dspy = SimpleNamespace(LM=MagicMock(side_effect=lambda *a, **kw: (a, kw)), context=context)
        self.ollama = SimpleNamespace(Client=MagicMock())
        dependencies = patch.dict(sys.modules, {"dspy": self.dspy, "ollama": self.ollama})
        dependencies.start()
        self.addCleanup(dependencies.stop)

    def test_defaults_are_fixed_not_discovered(self):
        self.assertEqual(ProviderConfig.from_env(), ProviderConfig("ollama", "llama3.2:3b", "http://localhost:11434", 120))
        self.ollama.Client.assert_not_called()
        self.dspy.LM.assert_not_called()

    def test_explicit_configuration_reaches_lm(self):
        with patch.dict(os.environ, {"ARCHGEN_AI_MODEL": "qwen3:8b", "ARCHGEN_OLLAMA_URL": "http://127.0.0.1:12345/", "ARCHGEN_AI_TIMEOUT_SECONDS": "45.5"}):
            result = with_local_provider(lambda: self.current_lm.get())()
        self.assertEqual(result[0], ("ollama_chat/qwen3:8b",))
        self.assertEqual(result[1]["api_base"], "http://127.0.0.1:12345")
        self.assertEqual(result[1]["timeout"], 45.5)
        self.assertEqual(result[1]["num_retries"], 0)
        self.assertFalse(result[1]["cache"])
        self.assertIsNone(self.current_lm.get())

    def test_invalid_configuration_never_calls_client(self):
        cases = {
            "ARCHGEN_AI_PROVIDER": ["openai", ""],
            "ARCHGEN_AI_MODEL": ["", "model with spaces"],
            "ARCHGEN_AI_TIMEOUT_SECONDS": ["0", "-1", "nan", "inf", "bad"],
            "ARCHGEN_OLLAMA_URL": ["https://example.com", "http://localhost.evil", "http://user:pass@localhost", "http://localhost/api", "http://localhost?x=1", "http://localhost:99999", "file:///tmp", ""],
        }
        for variable, values in cases.items():
            for value in values:
                with self.subTest(variable=variable, value=value), patch.dict(os.environ, {variable: value}):
                    with self.assertRaises(AIProviderError):
                        with_local_provider(lambda: None)()
        self.dspy.LM.assert_not_called()

    def test_ipv6_loopback_is_supported(self):
        with patch.dict(os.environ, {"ARCHGEN_OLLAMA_URL": "http://[::1]:11434"}):
            self.assertEqual(ProviderConfig.from_env().base_url, "http://[::1]:11434")

    def test_configuration_is_read_for_each_job(self):
        run = with_local_provider(lambda: self.current_lm.get()[0][0])
        self.assertEqual(run(), "ollama_chat/llama3.2:3b")
        with patch.dict(os.environ, {"ARCHGEN_AI_MODEL": "custom:v1"}):
            self.assertEqual(run(), "ollama_chat/custom:v1")

    def test_error_restores_context_and_does_not_retry(self):
        operation = MagicMock(side_effect=TimeoutError("private prompt text"))
        with self.assertRaisesRegex(AIProviderError, "timed out") as failure:
            with_local_provider(operation)()
        self.assertNotIn("private prompt", str(failure.exception))
        self.assertIsNone(self.current_lm.get())
        operation.assert_called_once()
        self.dspy.LM.assert_called_once()

    def test_generation_connection_and_unknown_errors_are_actionable(self):
        for error, message in [(ConnectionError("secret"), "ollama serve"), (ValueError("secret"), "requested output")]:
            with self.subTest(error=error), self.assertRaisesRegex(AIProviderError, message) as failure:
                with_local_provider(MagicMock(side_effect=error))()
            self.assertNotIn("secret", str(failure.exception))

    def test_health_check_uses_show_without_inference(self):
        result = health_check()
        self.assertEqual(result["status"], "ready")
        self.ollama.Client.assert_called_once_with(host="http://localhost:11434", timeout=120)
        self.ollama.Client.return_value.show.assert_called_once_with("llama3.2:3b")
        self.ollama.Client.return_value.chat.assert_not_called()
        self.ollama.Client.return_value.pull.assert_not_called()
        self.dspy.LM.assert_not_called()

    def test_missing_model_does_not_fallback(self):
        error = RuntimeError("private response")
        error.status_code = 404
        self.ollama.Client.return_value.show.side_effect = error
        with self.assertRaisesRegex(AIProviderError, "ollama pull"):
            health_check()
        self.ollama.Client.return_value.show.assert_called_once_with("llama3.2:3b")

    def test_concurrent_jobs_are_serialized_and_context_is_restored(self):
        first_entered, release_first, second_started, second_entered = (Event() for _ in range(4))

        @with_local_provider
        def first():
            first_entered.set()
            if not release_first.wait(3):
                raise AssertionError("test timed out")
            return self.current_lm.get()

        @with_local_provider
        def second():
            second_entered.set()
            return self.current_lm.get()

        def second_worker():
            second_started.set()
            value = second()
            return value, self.current_lm.get()

        with ThreadPoolExecutor(max_workers=2) as pool:
            one = pool.submit(first)
            try:
                self.assertTrue(first_entered.wait(3))
                two = pool.submit(second_worker)
                self.assertTrue(second_started.wait(3))
                self.assertFalse(second_entered.wait(0.05))
            finally:
                release_first.set()
            self.assertIsNotNone(one.result(timeout=3))
            value, after = two.result(timeout=3)
            self.assertIsNotNone(value)
            self.assertIsNone(after)

    def test_agent_import_and_public_entrypoints(self):
        # Import the real agent with dependency stubs. This exercises decorators
        # and the Rust-visible result shapes without importing DSPy disk caches.
        self.dspy.Signature = type("Signature", (), {})
        self.dspy.Module = type("Module", (), {})
        self.dspy.InputField = self.dspy.OutputField = MagicMock()
        self.dspy.Predict = self.dspy.ChainOfThought = MagicMock()
        self.dspy.configure = MagicMock()
        graph = SimpleNamespace(StateGraph=MagicMock(), END="end")
        with patch.dict(sys.modules, {"langgraph": SimpleNamespace(graph=graph), "langgraph.graph": graph}):
            sys.modules.pop("agent", None)
            try:
                agent = importlib.import_module("agent")
                self.dspy.LM.assert_not_called()
                self.dspy.configure.assert_not_called()
                self.ollama.Client.assert_not_called()
                with patch.object(agent, "diagram_gen", return_value="diagram") as generate:
                    self.assertEqual(agent.run_agent("request", "class"), {"content": "diagram", "format": "plantuml"})
                    generate.assert_called_once_with(system_description="request", diagram_type="class", output_format="plantuml", existing_diagram="")
                agent.agent_graph.invoke.return_value = {"sections": [{"title": "Overview"}]}
                self.assertEqual(agent.run_doc_agent("request", "arc42", language="sk"), {"template": "arc42", "sections": [{"title": "Overview"}]})
                self.assertEqual(agent.agent_graph.invoke.call_args.args[0]["language"], "sk")
                self.assertEqual(self.dspy.LM.call_count, 2)
            finally:
                sys.modules.pop("agent", None)


if __name__ == "__main__":
    unittest.main()
