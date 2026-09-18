# Network Flow Diagram

This diagram shows the HTTPS request flow for API calls through the Refactor Platform infrastructure.

```mermaid
flowchart TB
    Client[Client Browser]
    
    subgraph DO[Digital Ocean Droplet - backend_network]
        Nginx[Nginx Reverse Proxy<br/>:80, :443]
        NextJS[NextJS Frontend<br/>:3000]
        Axum[Axum Backend<br/>:4000]
        DocsCollab[docs-collab-server<br/>:1234 · PLANNED, not yet deployed]
    end
    
    Postgres[(D.O. Managed Postgres)]
    
    Client -.->|HTTPS/SSL :443| Nginx
    Nginx -->|:3000| NextJS
    NextJS -->|:4000| Axum
    Axum --> Postgres
    
    %% Collaboration: validated locally, production routing planned (not yet deployed)
    Nginx -.->|WebSocket collab :1234 PLANNED| DocsCollab
    Axum -.->|REST create/delete PLANNED| DocsCollab
    DocsCollab -.->|collab_documents PLANNED| Postgres
    
    classDef client fill:#e1f5fe
    classDef infrastructure fill:#f3e5f5
    classDef application fill:#e8f5e8
    classDef database fill:#fff3e0
    classDef planned fill:#f5f5f5,stroke:#9e9e9e,stroke-dasharray:5 5,color:#616161
    
    class Client client
    class Nginx infrastructure
    class NextJS,Axum application
    class Postgres database
    class DocsCollab planned
```

## Flow Description

1. **Client Request**: Web browser makes HTTPS requests (SSL/TLS encrypted) to the platform on port 443
2. **SSL Termination**: Nginx handles SSL termination at port 443 and routes traffic internally
3. **Frontend Processing**: Nginx forwards requests to NextJS frontend on port 3000
4. **API Forwarding**: NextJS forwards API calls to Axum backend on port 4000
5. **Backend Processing**: Axum backend handles API endpoints like `/api/login` with secure caching
6. **Database Operations**: Backend connects to Digital Ocean Managed PostgreSQL
7. **SSE Connections**: Long-lived `/api/sse` connections for real-time events (24h timeout, no buffering)

## Infrastructure Notes

- **Digital Ocean Droplet**: Hosts the containerized application stack (Nginx, NextJS, Axum) in the `backend_network` Docker network
- **Port Configuration**: 
  - Nginx: External ports 80/443, internal routing to services
  - NextJS: Internal port 3000 
  - Axum: Internal port 4000
- **Managed PostgreSQL**: Separate Digital Ocean managed database service, accessed over the internet with SSL
- **SSL/TLS**: HTTPS encryption from client to Nginx using Let's Encrypt certificates managed by `certbot`, then unencrypted internal traffic within the container network
- **Database Connection**: Axum connects to managed PostgreSQL over SSL outside the container network
- **SSE Configuration**: Nginx configured for long-lived connections (24h timeout, proxy buffering disabled) at `/api/sse` endpoint
- **Scaling Limitation**: SSE uses in-memory connection tracking - **single backend instance only** until Redis pub/sub is implemented
- **Collaboration (current vs planned)**: Collaborative notes today use **TipTap Cloud** (external SaaS reached over the internet, not shown in the droplet). The self-hosted **docs-collab-server** (Rust/Axum) has been validated locally (see `docs/test-plans/docs-collab-server-local-e2e.md`) and is shown above as a **planned** addition to `backend_network`: the frontend's collaboration WebSocket would route `Client -> Nginx -> docs-collab-server:1234`, the Axum backend would call its REST API to provision/delete documents, and it would persist Yjs state in a `collab_documents` table. Production deployment wiring (docker-compose, deploy workflow, TLS for the WS route) is **not yet done** and is out of scope until the rollout phase.