# RestApi Step

The `RestApi` step makes an HTTP/HTTPS call to an external service. It is a powerful intrinsic step that runs directly on the engine, without requiring a container or Kubernetes pod. It supports MiniJinja templating for the request body and allows extracting data from the response using JSON pointers or regular expressions.

## DSL Specification

The `spec` block for `RestApi` supports the following fields:

* `url` (string, required): The endpoint to send the request to. With a `connection`, a path such as `/invoices` is appended to the connection's `base_url`.
* `connection` (string, optional): The name of an `http_api` connection whose headers and bearer token are added to the request. See [Using a connection](#using-a-connection).
* `method` (string, optional): The HTTP method to use (e.g., "GET", "POST", "PUT"). Defaults to "GET".
* `headers` (object, optional): Key-value pairs of HTTP headers.
* `body` (string, optional): The request body. Supports MiniJinja template rendering with access to `inputs`, `steps`, and `run`.
* `timeout` (string, optional): Duration to wait for a response (e.g., "30s", "1m").
* `extractors` (list of objects, optional): Rules for extracting outputs from the response.

### Output Extractors

An output extractor allows you to capture parts of the response body and expose them as step outputs for subsequent steps to use.

An extractor object has the following fields:

* `name` (string, required): The name of the output variable to create.
* `format` (string, optional): The format of the extraction. Can be `"json"` or `"regex"`.
* `json_pointer` (string, optional): If `format` is `"json"`, this field acts as a **JSON Pointer** (e.g., `"/data/items/0/id"`) or dot-notation path (e.g., `"data.items.0.id"`).
* `regex` (string, optional): If `format` is `"regex"`, this field is the regular expression to match against the response body.
* `group` (number, optional): If `format` is `"regex"`, the capture group index to extract (defaults to 1).
* `sensitive` (boolean, optional): Marks the extracted output as sensitive when it is persisted.

If no extractors are provided, or regardless of the extractors, the entire parsed response body is automatically captured in an output named `response`.

> [!WARNING]
> The `response` output always includes the full parsed body and is not automatically marked sensitive. If the response may include secrets, avoid persisting/printing `response` directly and use `extractors` with `sensitive = true` for the specific fields that need to flow to later steps.

## Using a connection

An `http_api` connection supplies a `base_url`, optional `headers` and an optional bearer token, so the workflow does not have to carry the credential itself.

The connection's headers and token are only sent to the connection's own server. The request URL must be a path relative to `base_url`, or an absolute URL with the same scheme, host and port as `base_url`; any other URL fails the step before a request is made. A connection that has headers or a token but no `base_url` cannot be used by this step. A connection with neither places no restriction on the URL.

```hcl
step "list_invoices" "RestApi" {
  spec = {
    connection = "billing-api"
    url        = "/invoices?status=open"
  }
}
```

## Example

```hcl
step "fetch_user_data" "RestApi" {
  spec = {
    url    = "https://api.example.com/users/{{ inputs.user_id }}"
    method = "GET"
    headers = {
      "Accept"        = "application/json"
      "Authorization" = "Bearer ${secrets.API_TOKEN}"
    }
    timeout = "15s"

    extractors = [
      {
        name         = "user_email"
        format       = "json"
        json_pointer = "/data/email"
      },
      {
        name      = "session_token"
        format    = "regex"
        regex     = "Token is ([A-Z0-9]+)"
        group     = 1
        sensitive = true
      }
    ]
  }
}
```
