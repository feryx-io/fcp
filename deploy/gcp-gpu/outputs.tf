output "instance" {
  description = "The VM's name, or null while it is down."
  value       = var.up ? google_compute_instance.gpu[0].name : null
}

output "zone" {
  value = var.zone
}

output "ssh" {
  description = "How to reach it (through IAP; no public SSH port)."
  value       = var.up ? "gcloud compute ssh ${var.name} --project ${var.project} --zone ${var.zone} --tunnel-through-iap" : "down (terraform apply -var up=true)"
}

output "disk" {
  description = "The persistent disk that outlives the VM."
  value       = google_compute_disk.boot.name
}
