# System Architecture Overview

The Refactor Platform is a coaching management system built with Rust (Axum backend) and Next.js (frontend). This diagram shows the internal application architecture and how requests flow through the system.

## Key Architecture Principles

- **Layered Architecture**: Clear separation between web, business logic, and data layers
- **Domain-Driven Design**: Core business logic centralized in the domain layer
- **Repository Pattern**: Entity API layer abstracts database operations
- **Dependency Injection**: Service layer provides configuration and utilities
- **Authentication**: Session-based auth with JWT token support

## System Components

### External Layer
- **Client**: Web frontend (Next.js) and potential API clients
- **Nginx**: Reverse proxy handling HTTPS termination and load balancing

### Web Layer (Axum HTTP Server)
- **main.rs**: Application entry point, bootstraps the server
- **Router**: Defines API routes and applies middleware (auth, CORS, logging)
- **Controllers**: Handle HTTP requests, validate input, call business logic
- **Authentication**: Manages user sessions and request authorization

### Business Logic Layer
- **Domain**: Core business models and logic (Users, Organizations, Coaching Sessions, etc.)
- **Entity API**: Database operations abstraction layer
- **Service**: Configuration management, logging, utilities

### Data Layer
- **Entity**: Database models using SeaORM
- **Migration**: Database schema versioning and migrations
- **Database**: PostgreSQL with `refactor_platform` schema

### Real-Time Communication
- **SSE (Server-Sent Events)**: Unidirectional push notifications from server to client
- **Connection Management**: In-memory registry for active user connections (single-instance only)

### External Integrations
- **TipTap**: Collaborative document editing service
- **JWT**: Token generation and validation service
- **MailerSend**: Transactional email service for notifications

## Data Flow Example

1. **HTTP Request** → Nginx → Axum Web Server
2. **Routing** → Router matches URL to controller
3. **Authentication** → Middleware validates session/token
4. **Controller** → Validates input, calls domain logic
5. **Domain** → Implements business rules, calls Entity API
6. **Entity API** → Performs database operations via Entity models
7. **Response** → Results flow back through the layers

## Core Business Entities

- **Users**: Coaches and coachees in the system
- **Organizations**: Groups that manage coaching relationships
- **Coaching Relationships**: Connections between coaches and coachees
- **Coaching Sessions**: Individual coaching meetings with notes and goals
- **Actions**: Commitments and next steps from sessions
- **Agreements**: Formal coaching agreements and contracts
- **Overarching Goals**: Long-term objectives for coaching relationships

```mermaid
graph TB
    %% External Layer
    Client[Web Frontend/API Client]
    Nginx[Nginx Reverse Proxy<br/>HTTPS Termination]
    
    %% Application Layer
    Main[main.rs<br/>Application Entry Point]
    Web[Web Layer<br/>Axum HTTP Server]
    
    %% Core Components
    Router[Router<br/>Route Definitions & Middleware]
    Controllers[Controllers<br/>HTTP Request Handlers]
    Auth[Authentication Layer<br/>Session Management]
    SSE[SSE Handler<br/>Real-Time Events]

    %% Business Logic Layer
    Domain[Domain Layer<br/>Business Logic & Models]
    EntityAPI[Entity API<br/>Database Operations]
    Service[Service Layer<br/>Configuration & Utilities]
    SSEManager[SSE Manager<br/>Connection Registry]
    
    %% Data Layer
    Entity[Entity Layer<br/>Database Models]
    Migration[Migration<br/>Database Schema]
    DB[(PostgreSQL Database<br/>refactor_platform schema)]
    
    %% External Services
    TipTap[TipTap Gateway<br/>Document Collaboration]
    JWT[JWT Service<br/>Token Generation]
    MailerSend[MailerSend<br/>Email Service]
    
    %% Request Flow
    Client --> Nginx
    Nginx --> Web
    Web --> Main
    Main --> Router
    
    %% Router to Controllers
    Router --> Controllers
    Router --> Auth
    Router --> SSE
    
    %% Controllers breakdown
    Controllers --> ActionCtrl[Action Controller]
    Controllers --> AgreementCtrl[Agreement Controller]
    Controllers --> CoachingCtrl[Coaching Session Controller]
    Controllers --> NoteCtrl[Note Controller]
    Controllers --> OrgCtrl[Organization Controller]
    Controllers --> UserCtrl[User Controller]
    Controllers --> GoalCtrl[Overarching Goal Controller]
    Controllers --> SessionCtrl[User Session Controller]
    Controllers --> JWTCtrl[JWT Controller]
    Controllers --> HealthCtrl[Health Check Controller]
    
    %% Business Logic Flow
    ActionCtrl --> Domain
    AgreementCtrl --> Domain
    CoachingCtrl --> Domain
    NoteCtrl --> Domain
    OrgCtrl --> Domain
    UserCtrl --> Domain
    GoalCtrl --> Domain
    SessionCtrl --> Domain
    JWTCtrl --> Domain
    
    %% Domain to Data Access
    Domain --> EntityAPI
    Domain --> Service

    %% SSE Integration
    SSE --> SSEManager
    Service --> SSEManager
    Domain -.->|send events| SSEManager
    
    %% Data Access Layer
    EntityAPI --> Entity
    Service --> Entity
    Entity --> DB
    Migration --> DB
    
    %% Authentication Flow
    Auth --> Domain
    Auth -.-> DB
    
    %% External Integrations
    Domain --> TipTap
    Domain --> JWT
    Domain --> MailerSend
    
    %% Styling
    classDef external fill:#e1f5fe
    classDef web fill:#f3e5f5
    classDef business fill:#e8f5e8
    classDef data fill:#fff3e0
    classDef database fill:#ffebee
    
    class Client,Nginx external
    class Web,Router,Controllers,Auth,SSE,ActionCtrl,AgreementCtrl,CoachingCtrl,NoteCtrl,OrgCtrl,UserCtrl,GoalCtrl,SessionCtrl,JWTCtrl,HealthCtrl web
    class Domain,EntityAPI,Service,SSEManager business
    class Entity,Migration data
    class DB database
```