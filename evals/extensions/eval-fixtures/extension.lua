local object = { type = "object", additionalProperties = false, properties = {} }
habibi.tools.register({ name = "eval.immediate", description = "Return a deterministic evaluation marker immediately.", input_schema = object }, function()
  return { result = { ok = true, marker = "immediate" } }
end)
habibi.tools.register({ name = "eval.marker", description = "Emit and return a deterministic evaluation marker.", input_schema = object }, function()
  return { result = { ok = true, marker = "visible" }, events = {{ type = "eval.marker.created", payload = { marker = "visible" } }} }
end)
habibi.tools.register({ name = "eval.fail", description = "Fail deterministically for evaluation of action failures.", input_schema = object }, function()
  error("intentional eval failure")
end)
habibi.tools.register({ name = "eval.malformed", description = "Return an invalid effect namespace for failure isolation evaluation.", input_schema = object }, function()
  return { result = { unreachable = true }, events = {{ type = "outside.invalid", payload = {} }} }
end)
habibi.tools.suggest("eval.fixture-tools", function(event)
  if event.event_type ~= "chat.session.started" and event.event_type ~= "chat.message.created" then return habibi.array({}) end
  local content = string.lower(event.payload.content or "")
  local suggestions = habibi.array({})
  if string.find(content, "eval", 1, true) or string.find(content, "multiple", 1, true) or string.find(content, "mixed", 1, true) then
    for _, tool in ipairs({ "eval.immediate", "eval.marker", "eval.fail", "eval.malformed" }) do
      table.insert(suggestions, { tool = tool, reason = "Requested by an isolated evaluation fixture." })
    end
  end
  return suggestions
end)
