local function error_response(status, message)
  return { status = status, json = { error = message } }
end

local function chat_events(limit)
  return habibi.events.query({ prefix = "chat.", limit = limit or 1000 })
end

local function query_all(event_type)
  local result = habibi.array({})
  local before_sequence = nil
  while true do
    local page = habibi.events.query({
      type = event_type,
      before_sequence = before_sequence,
      limit = 1000
    })
    for index = #page, 1, -1 do
      table.insert(result, 1, page[index])
    end
    if #page < 1000 then break end
    before_sequence = page[1].sequence
  end
  return result
end

local function object_body(request)
  if type(request.json) ~= "table" then return nil end
  return request.json
end

local function project_sessions()
  local sessions = {}
  local events = habibi.array({})
  local event_types = {
    "chat.session.created",
    "chat.session.renamed",
    "chat.session.archived",
    "chat.message.created"
  }
  for _, event_type in ipairs(event_types) do
    for _, event in ipairs(query_all(event_type)) do
      table.insert(events, event)
    end
  end
  table.sort(events, function(a, b) return a.sequence < b.sequence end)

  for _, event in ipairs(events) do
    local payload = event.payload or {}
    local id = payload.session_id
    if event.event_type == "chat.session.created" and id then
      sessions[id] = {
        id = id,
        title = payload.title or "New chat",
        archived = false,
        created_at = event.occurred_at,
        updated_at = event.occurred_at,
        created_sequence = event.sequence
      }
    elseif event.event_type == "chat.session.renamed" and sessions[id] then
      sessions[id].title = payload.title or sessions[id].title
      sessions[id].updated_at = event.occurred_at
    elseif event.event_type == "chat.session.archived" and sessions[id] then
      sessions[id].archived = payload.archived ~= false
      sessions[id].updated_at = event.occurred_at
    elseif event.event_type == "chat.message.created" and sessions[id] then
      sessions[id].updated_at = event.occurred_at
      sessions[id].last_message = payload.content
    end
  end

  local result = habibi.array({})
  for _, session in pairs(sessions) do
    table.insert(result, session)
  end
  table.sort(result, function(a, b)
    return (a.updated_at or "") > (b.updated_at or "")
  end)
  return result
end

local function find_session(session_id)
  for _, session in ipairs(project_sessions()) do
    if session.id == session_id then return session end
  end
  return nil
end

local function session_messages(session_id)
  local messages = habibi.array({})
  for _, event in ipairs(query_all("chat.message.created")) do
    local payload = event.payload or {}
    if payload.session_id == session_id then
      table.insert(messages, {
        id = payload.message_id,
        session_id = session_id,
        role = payload.role,
        content = payload.content,
        created_at = event.occurred_at,
        event_id = event.id,
        sequence = event.sequence
      })
    end
  end
  return messages
end

habibi.web.route("GET", "/api/sessions", function(_request)
  return { status = 200, json = project_sessions() }
end)

habibi.web.route("POST", "/api/sessions", function(request)
  local body = object_body(request)
  if not body then return error_response(400, "request body must be a JSON object") end
  local session_id = habibi.id()
  local title = body.title or "New chat"
  return {
    status = 201,
    json = { id = session_id, title = title },
    emit = {
      type = "chat.session.created",
      payload = { session_id = session_id, title = title }
    }
  }
end)

habibi.web.route("GET", "/api/sessions/:session_id", function(request)
  local session = find_session(request.path_params.session_id)
  if not session then return error_response(404, "session not found") end
  return { status = 200, json = session }
end)

habibi.web.route("PATCH", "/api/sessions/:session_id", function(request)
  local session_id = request.path_params.session_id
  if not find_session(session_id) then return error_response(404, "session not found") end
  local body = object_body(request)
  if not body then return error_response(400, "request body must be a JSON object") end
  if body.title then
    return {
      status = 200,
      json = { id = session_id, title = body.title },
      emit = {
        type = "chat.session.renamed",
        payload = { session_id = session_id, title = body.title }
      }
    }
  end
  return error_response(400, "nothing to update")
end)

habibi.web.route("DELETE", "/api/sessions/:session_id", function(request)
  local session_id = request.path_params.session_id
  if not find_session(session_id) then return error_response(404, "session not found") end
  return {
    status = 200,
    json = { id = session_id, archived = true },
    emit = {
      type = "chat.session.archived",
      payload = { session_id = session_id, archived = true }
    }
  }
end)

habibi.web.route("GET", "/api/sessions/:session_id/messages", function(request)
  local session_id = request.path_params.session_id
  if not find_session(session_id) then return error_response(404, "session not found") end
  return { status = 200, json = session_messages(session_id) }
end)

habibi.web.route("POST", "/api/sessions/:session_id/messages", function(request)
  local session_id = request.path_params.session_id
  if not find_session(session_id) then return error_response(404, "session not found") end
  local body = object_body(request)
  if not body then return error_response(400, "request body must be a JSON object") end
  local content = body.content
  if type(content) ~= "string" or content:match("^%s*$") then
    return error_response(400, "content must be a non-empty string")
  end

  return {
    status = 201,
    json = { session_id = session_id },
    emit = {
      type = "chat.message.created",
      payload = {
        session_id = session_id,
        message_id = habibi.id(),
        role = "user",
        content = content
      }
    },
    react = true
  }
end)

habibi.web.route("GET", "/api/events", function(_request)
  return { status = 200, json = chat_events() }
end)

habibi.web.route("GET", "/api/preferences", function(_request)
  return { status = 200, json = habibi.kv.get("preferences") or {} }
end)

habibi.web.route("PUT", "/api/preferences", function(request)
  local preferences = object_body(request)
  if not preferences then return error_response(400, "request body must be a JSON object") end
  habibi.kv.set("preferences", preferences)
  return { status = 200, json = preferences }
end)

habibi.tools.register({
  name = "chat.get_sessions",
  description = "List chat sessions or retrieve one specific session. Chat sessions organize the UI but do not isolate Habibi's global memory.",
  input_schema = {
    type = "object",
    properties = {
      session_id = { type = "string" },
      include_archived = { type = "boolean" },
      limit = { type = "integer", minimum = 1, maximum = 100 }
    }
  },
  continuation = "required"
}, function(arguments, _context)
  if arguments.session_id then
    return { result = { session = find_session(arguments.session_id) } }
  end
  local result = habibi.array({})
  local limit = arguments.limit or 50
  for _, session in ipairs(project_sessions()) do
    if arguments.include_archived or not session.archived then
      table.insert(result, session)
      if #result >= limit then break end
    end
  end
  return { result = { sessions = result } }
end)

habibi.tools.register({
  name = "chat.search_messages",
  description = "Search chat messages for a case-insensitive keyword across one session or all sessions.",
  input_schema = {
    type = "object",
    properties = {
      query = { type = "string" }, session_id = { type = "string" },
      role = { type = "string", enum = { "user", "assistant" } },
      limit = { type = "integer", minimum = 1, maximum = 100 }
    },
    required = { "query" }
  },
  continuation = "required"
}, function(arguments, _context)
  local needle = string.lower(arguments.query or "")
  local matches = habibi.array({})
  local limit = arguments.limit or 20
  local events = query_all("chat.message.created")
  for index = #events, 1, -1 do
    local event = events[index]
    local payload = event.payload or {}
    if (not arguments.session_id or payload.session_id == arguments.session_id)
      and (not arguments.role or payload.role == arguments.role)
      and string.find(string.lower(payload.content or ""), needle, 1, true) then
      table.insert(matches, {
        event_id = event.id, sequence = event.sequence, occurred_at = event.occurred_at,
        session_id = payload.session_id, message_id = payload.message_id,
        role = payload.role, content = payload.content
      })
      if #matches >= limit then break end
    end
  end
  return { result = { messages = matches } }
end)

habibi.tools.register({
  name = "chat.send_message",
  description = "Send a message to the user in a chat session. Use this tool for every user-visible response. Defaults to the triggering session.",
  input_schema = {
    type = "object",
    properties = { session_id = { type = "string" }, content = { type = "string" } },
    required = { "content" }
  },
  continuation = "terminal"
}, function(arguments, context)
  local session_id = arguments.session_id or context.trigger.payload.session_id
  if not session_id or not find_session(session_id) then error("chat session not found") end
  if type(arguments.content) ~= "string" or arguments.content:match("^%s*$") then error("content must be non-empty") end
  local message_id = habibi.id()
  return {
    result = { sent = true, session_id = session_id, message_id = message_id },
    events = {{
      type = "chat.message.created",
      payload = { session_id = session_id, message_id = message_id, role = "assistant", content = arguments.content }
    }}
  }
end)

habibi.reactions.context(function(trigger)
  local session_id = trigger.payload.session_id
  local messages = session_messages(session_id)
  local context = habibi.array({})
  local first = math.max(1, #messages - 39)
  for index = first, #messages do
    table.insert(context, {
      role = messages[index].role,
      content = messages[index].content
    })
  end
  return context
end)
