-- Personas and nudges for user voice/style customization

-- System personas (predefined templates)
CREATE TABLE personas (
    id BIGSERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    nudges TEXT NOT NULL,
    is_system BOOLEAN DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- User's custom personas (saved from edited nudges)
CREATE TABLE user_personas (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    nudges TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX idx_user_personas_user ON user_personas(user_id);

-- User's active nudges configuration
ALTER TABLE users ADD COLUMN nudges TEXT;
ALTER TABLE users ADD COLUMN selected_persona_id BIGINT REFERENCES personas(id);

-- Seed system personas
INSERT INTO personas (name, slug, nudges, is_system) VALUES
(
    'Indie Builder',
    'indie_builder',
    'I write casually, usually lowercase. I share what I''m building - progress updates, metrics when they''re interesting, the behind-the-scenes stuff. I''m direct about things, I don''t hedge or over-explain. Screenshots of dashboards, code, whatever I''m actually looking at. No corporate speak, no buzzwords.',
    true
),
(
    'Technical',
    'technical',
    'I write about code and technical problems I''m working through. I like showing my thinking - why I made a choice, what surprised me, what I learned. I explain things but I don''t talk down to people. If I use jargon I make sure it makes sense in context.',
    true
),
(
    'Conversational',
    'conversational',
    'I''m here to make observations and joke around. Self-deprecating humor, absurd takes on normal situations. I find unexpected angles on things. I don''t try too hard and I''m not mean about it.',
    true
);
