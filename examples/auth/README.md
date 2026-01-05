# Xitca Web Authentication Example

A high-performance, asynchronous Web API for User Authentication (Login & Register) built with the **Xitca-web** framework and **Xitca-postgres** driver. This project demonstrates best practices in Rust web development, including password hashing, request validation, and shared state management.

## 🚀 Features

* **Xitca-web**: An ultra-fast, service-oriented web framework.
* **Xitca-postgres**: A non-blocking PostgreSQL driver with zero-copy parsing.
* **Secure Auth**: Password hashing using `password-auth`.
* **CUID2**: Secure, collision-resistant unique identifiers for User IDs.
* **Validation**: Robust input validation using the `validator` crate.
* **Middleware**: Built-in Rate Limiting and Payload Compression.

## 🛠 Prerequisites

Before running this project, ensure you have the following installed:

* **Rust** (latest stable version)
* **PostgreSQL** (running locally or via Docker)
* **Cargo** (Rust package manager)

## 📥 Installation & Setup

### 1. Clone the Repository

```bash
git clone https://github.com/your-username/your-repo-name.git
cd your-repo-name

```

### 2. Configure Environment Variables

Create a `.env` file in the root directory and add your database credentials:

```env
DATABASE_URL=postgres://username:password@localhost:5432/your_database_name

```

### 3. Database Schema

Ensure your PostgreSQL database has a `users` table. You can run the following SQL:

```sql
CREATE TABLE users (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    email TEXT UNIQUE NOT NULL,
    password TEXT NOT NULL
);

```

## 🏃 Running the Application

### Development Mode

To start the server with auto-reload (requires `cargo-watch`):

```bash
cargo watch -x run

```

### Standard Run

```bash
cargo run

```

The server will start at `http://localhost:8080`.

## 📡 API Endpoints

| Method | Endpoint | Description |
| --- | --- | --- |
| `POST` | `/register` | Register a new user account |
| `POST` | `/login` | Authenticate existing user |

### Example Request (Register)

**Payload:**

```json
{
  "name": "John Doe",
  "email": "john@example.com",
  "password": "securepassword123"
}

```

## 📂 Project Structure

* `src/main.rs`: Entry point and server configuration.
* `src/db/`: Database client and connection management.
* `src/routes/`: Request handlers for login and registration.
* `src/utils/`: Helper functions for JSON responses and validation.
