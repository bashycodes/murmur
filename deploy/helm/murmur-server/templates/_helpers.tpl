{{- define "flow-server.name" -}}
{{- .Chart.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}

{{- define "flow-server.fullname" -}}
{{- if contains .Chart.Name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "flow-server.labels" -}}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" }}
{{ include "flow-server.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{- define "flow-server.selectorLabels" -}}
app.kubernetes.io/name: {{ include "flow-server.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{/* config.toml: values.config, plus [tls] when serverTls is on. */}}
{{- define "flow-server.config" -}}
{{ .Values.config }}
{{- if .Values.serverTls.enabled }}

[tls]
cert = "/etc/flow-server/tls/tls.crt"
key = "/etc/flow-server/tls/tls.key"
{{- end }}
{{- end -}}

{{- define "flow-server.image" -}}
{{- $img := .Values.image -}}
{{- if .Values.gpu.enabled -}}{{- $img = merge (dict) .Values.gpu.image .Values.image -}}{{- end -}}
{{- printf "%s:%s" $img.repository $img.tag -}}
{{- end -}}
