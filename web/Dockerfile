# Build stage
FROM node:22-alpine AS builder

WORKDIR /app

# Copy package files
COPY package.json package-lock.json ./

# Install dependencies
RUN npm ci

# Copy source
COPY . .

# Build for production
RUN npm run build

# Runtime stage
FROM node:22-alpine

WORKDIR /app

# Copy package files for preview server
COPY package.json package-lock.json ./

# Install only production deps + vite for preview
RUN npm ci

# Copy built assets and vite config
COPY --from=builder /app/dist ./dist
COPY vite.config.ts ./

EXPOSE 4173

CMD ["npm", "run", "preview", "--", "--host", "0.0.0.0"]
