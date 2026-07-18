ALTER TABLE model_providers
  DROP CONSTRAINT IF EXISTS model_providers_provider_kind_check;

ALTER TABLE model_providers
  ADD CONSTRAINT model_providers_provider_kind_check
  CHECK (provider_kind IN ('openai', 'deepseek', 'ollama', 'llama-cpp', 'openai-compatible'));
