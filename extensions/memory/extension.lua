local MAX_CAUSAL_EVENTS = 20
local MAX_SEMANTIC_EVENTS = 20
local MINIMUM_SIMILARITY = 0.50
local MAX_QUERY_BYTES = 16 * 1024

local function retrieve(trigger)
  local causal_newest_first = habibi.array({})
  local seen = {}
  local event_id = trigger.causation_id

  while type(event_id) == "string" and #causal_newest_first < MAX_CAUSAL_EVENTS do
    if seen[event_id] then break end
    local event = habibi.events.get(event_id)
    if not event then break end
    seen[event_id] = true
    table.insert(causal_newest_first, event)
    event_id = event.causation_id
  end

  local causal = habibi.array({})
  for index = #causal_newest_first, 1, -1 do
    table.insert(causal, causal_newest_first[index])
  end

  local referenced = habibi.array({})
  local result_ids = trigger.payload.result_event_ids
  if type(result_ids) == "table" then
    for _, id in ipairs(result_ids) do
      if type(id) == "string" and not seen[id] then
        local event = habibi.events.get(id)
        if event then
          seen[id] = true
          table.insert(referenced, event)
        end
      end
    end
  end

  local semantic = habibi.array({})
  local semantic_metadata = nil
  local stored_trigger = habibi.events.get(trigger.id)
  if stored_trigger then
    local query = trigger.event_type .. "\n" .. habibi.json.encode(trigger.payload)
    if #query > MAX_QUERY_BYTES then query = query:sub(1, MAX_QUERY_BYTES) end
    local ok, matches = pcall(habibi.events.semantic, {
      text = query,
      before_sequence = stored_trigger.sequence,
      limit = MAX_SEMANTIC_EVENTS,
      minimum_similarity = MINIMUM_SIMILARITY
    })
    if ok then
      semantic_metadata = {
        embedding_model = matches.embedding_model,
        embedding_revision = matches.embedding_revision,
        candidates_scanned = matches.candidates_scanned
      }
      for _, match in ipairs(matches.matches) do
        local id = match.event.id
        if not seen[id] then
          seen[id] = true
          table.insert(semantic, match)
        end
      end
    end
  end

  if #causal == 0 and #referenced == 0 and #semantic == 0 then return { content = "" } end
  return {
    content = habibi.json.encode({
      memory = {
        causal = causal,
        referenced = referenced,
        semantic = semantic,
        semantic_metadata = semantic_metadata
      }
    })
  }
end

habibi.context.register("retrieve", retrieve)
