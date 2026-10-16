{{/*
Expand the name of the chart.
*/}}
{{- define "stormchaser-orchestration.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Create a default fully qualified app name.
We truncate at 63 chars because some Kubernetes name fields are limited to this (by the DNS naming spec).
If release name contains chart name it will be used as a full name.
*/}}
{{- define "stormchaser-orchestration.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{/*
Create chart name and version as used by the chart label.
*/}}
{{- define "stormchaser-orchestration.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{/*
Common labels
*/}}
{{- define "stormchaser-orchestration.labels" -}}
helm.sh/chart: {{ include "stormchaser-orchestration.chart" . }}
{{ include "stormchaser-orchestration.selectorLabels" . }}
{{- if .Chart.AppVersion }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
{{- end }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{/*
Selector labels
*/}}
{{- define "stormchaser-orchestration.selectorLabels" -}}
app.kubernetes.io/name: {{ include "stormchaser-orchestration.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{/*
"true" when the API, the query service and the web tier serve HTTPS themselves. The
services serve HTTPS unless told otherwise, so this is always passed to them.
*/}}
{{- define "stormchaser-orchestration.listenTls" -}}
{{- if .Values.global.tls.listen -}}
{{- if not .Values.global.tls.enabled -}}
{{- fail "global.tls.listen needs a certificate: set global.tls.enabled=true and name its Secret in global.tls.existingSecret, or set global.tls.listen=false where something in front of the services terminates TLS and the network behind it is trusted" -}}
{{- end -}}
true
{{- else -}}
false
{{- end -}}
{{- end }}

{{/*
The scheme the web tier uses to reach the API and the query service.
*/}}
{{- define "stormchaser-orchestration.internalScheme" -}}
{{- if eq (include "stormchaser-orchestration.listenTls" .) "true" -}}https{{- else -}}http{{- end -}}
{{- end }}

{{/*
Ingress annotations, with the backend protocol set to HTTPS for ingress-nginx when the
service behind it serves HTTPS. Call with (dict "annotations" ... "root" $).
*/}}
{{- define "stormchaser-orchestration.ingressAnnotations" -}}
{{- $annotations := deepCopy (.annotations | default dict) -}}
{{- if eq (include "stormchaser-orchestration.listenTls" .root) "true" -}}
{{- $_ := set $annotations "nginx.ingress.kubernetes.io/backend-protocol" (index $annotations "nginx.ingress.kubernetes.io/backend-protocol" | default "HTTPS") -}}
{{- end -}}
{{- with $annotations }}
annotations:
  {{- toYaml . | nindent 2 }}
{{- end }}
{{- end }}

{{/*
The OPA sidecar. The API, the engine and the query service each run one and reach it on
loopback, so the bearer tokens they send it with each decision never cross the network
and nothing outside the pod can ask it anything.
*/}}
{{- define "stormchaser-orchestration.opaSidecar" -}}
{{- if .Values.api.opa.sidecar.enabled }}
- name: opa
  image: "{{ .Values.api.opa.sidecar.image.repository }}:{{ .Values.api.opa.sidecar.image.tag }}"
  args:
    - "run"
    - "--server"
    - "--addr=127.0.0.1:8181"
    # Limits OPA's own API to evaluating the policy, so nothing else in the pod can
    # replace it
    - "--authorization=basic"
    - "/config/policy.rego"
    - "/config/authz.rego"
    - "/config/settings.json"
  securityContext:
    {{- toYaml .Values.securityContext | nindent 4 }}
  volumeMounts:
    - name: opa-policy
      mountPath: /config
      readOnly: true
  resources:
    {{- toYaml .Values.api.opa.sidecar.resources | nindent 4 }}
{{- end }}
{{- end }}

{{/*
The volume holding the policy the OPA sidecar loads.
*/}}
{{- define "stormchaser-orchestration.opaPolicyVolume" -}}
{{- if .Values.api.opa.sidecar.enabled }}
- name: opa-policy
  configMap:
    name: {{ include "stormchaser-orchestration.fullname" . }}-opa-policy
{{- end }}
{{- end }}

{{/*
The Secret holding credentials that were given as values.
*/}}
{{- define "stormchaser-orchestration.credentialsSecret" -}}
{{ include "stormchaser-orchestration.fullname" . }}-credentials
{{- end }}

{{/*
The DATABASE_URL environment variable, from the named Secret or the chart's own.
*/}}
{{- define "stormchaser-orchestration.databaseUrlEnv" -}}
- name: DATABASE_URL
  valueFrom:
    secretKeyRef:
      {{- if .Values.global.database.existingSecret }}
      name: {{ .Values.global.database.existingSecret }}
      key: {{ .Values.global.database.existingSecretKey }}
      {{- else }}
      name: {{ include "stormchaser-orchestration.credentialsSecret" . }}
      key: DATABASE_URL
      {{- end }}
{{- end }}

{{/*
Refuses a database URL with no host, or with the password this chart once shipped as
its default, which is public.
*/}}
{{- define "stormchaser-orchestration.checkDatabaseUrl" -}}
{{- $url := .Values.global.database.url | default "" -}}
{{- if not $url -}}
{{- fail "global.database.url is empty: set it, or name a Secret holding it in global.database.existingSecret" -}}
{{- end -}}
{{- if regexMatch "^[a-z]+://[^:/@]*:stormchaser@" $url -}}
{{- fail "global.database.url uses the password \"stormchaser\", which was this chart's public default: choose another, preferably in a Secret named by global.database.existingSecret" -}}
{{- end -}}
{{- if contains "CHANGE_ME_IN_PRODUCTION" $url -}}
{{- fail "global.database.url uses the placeholder password the umbrella chart once shipped, which is public: choose another, preferably in a Secret named by global.database.existingSecret" -}}
{{- end -}}
{{- end }}
