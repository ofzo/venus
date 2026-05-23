use anyhow::{Context, Result};
use chrono::{Datelike, Local, Timelike};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{info, warn};

// ---- Cron Expression Parser ----

/// Parsed 5-field cron expression: minute hour day-of-month month day-of-week
#[derive(Debug, Clone)]
pub struct CronExpr {
    minutes: Vec<u32>,       // 0-59
    hours: Vec<u32>,         // 0-23
    days_of_month: Vec<u32>, // 1-31
    months: Vec<u32>,        // 1-12
    days_of_week: Vec<u32>,  // 0-6 (0=Sunday)
}

impl CronExpr {
    /// Parse a 5-field cron expression.
    /// Supports: * (wildcard), N (specific), N-M (range), */N (step), N,M (list)
    pub fn parse(expr: &str) -> Result<Self> {
        let fields: Vec<&str> = expr.split_whitespace().collect();
        if fields.len() != 5 {
            anyhow::bail!(
                "cron expression must have exactly 5 fields, got {}",
                fields.len()
            );
        }

        Ok(Self {
            minutes: Self::parse_field(fields[0], 0, 59).context("invalid minute field")?,
            hours: Self::parse_field(fields[1], 0, 23).context("invalid hour field")?,
            days_of_month: Self::parse_field(fields[2], 1, 31)
                .context("invalid day-of-month field")?,
            months: Self::parse_field(fields[3], 1, 12).context("invalid month field")?,
            days_of_week: Self::parse_field(fields[4], 0, 6)
                .context("invalid day-of-week field")?,
        })
    }

    /// Check if a datetime matches this cron expression.
    pub fn matches(&self, dt: &chrono::DateTime<Local>) -> bool {
        self.minutes.contains(&dt.minute())
            && self.hours.contains(&dt.hour())
            && self.days_of_month.contains(&dt.day())
            && self.months.contains(&dt.month())
            && self.days_of_week.contains(&dt.weekday().num_days_from_sunday())
    }

    /// Parse a single cron field. Handles: *, N, N-M, */N, N-M/S, N,M,O
    fn parse_field(field: &str, min: u32, max: u32) -> Result<Vec<u32>> {
        let mut values = Vec::new();

        for part in field.split(',') {
            let part = part.trim();

            if part.contains('/') {
                // Step: */N or N-M/S
                let parts: Vec<&str> = part.splitn(2, '/').collect();
                let step: u32 = parts[1].parse().context("invalid step value")?;
                if step == 0 {
                    anyhow::bail!("step cannot be 0");
                }
                let (start, end) = if parts[0] == "*" {
                    (min, max)
                } else if parts[0].contains('-') {
                    let range: Vec<&str> = parts[0].splitn(2, '-').collect();
                    let s: u32 = range[0].parse()?;
                    let e: u32 = range[1].parse()?;
                    (s, e)
                } else {
                    let s: u32 = parts[0].parse()?;
                    (s, max)
                };
                let mut v = start;
                while v <= end {
                    values.push(v);
                    v += step;
                }
            } else if part == "*" {
                values.extend(min..=max);
            } else if part.contains('-') {
                // Range: N-M
                let range: Vec<&str> = part.splitn(2, '-').collect();
                let start: u32 = range[0].parse()?;
                let end: u32 = range[1].parse()?;
                if start > end {
                    anyhow::bail!("range start {} > end {}", start, end);
                }
                values.extend(start..=end);
            } else {
                // Specific value
                let v: u32 = part.parse().context("invalid numeric value")?;
                if v < min || v > max {
                    anyhow::bail!("value {} out of range {}-{}", v, min, max);
                }
                values.push(v);
            }
        }

        values.sort();
        values.dedup();
        if values.is_empty() {
            anyhow::bail!("field produced no values");
        }
        Ok(values)
    }
}

// ---- Cron Job ----

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub cron_expr: String,
    pub prompt: String,
    pub recurring: bool,
    pub durable: bool,
    pub created_at: u64,
    pub last_fired: Option<u64>,
    pub expires_at: u64,
}

// ---- Cron Scheduler ----

pub struct CronScheduler {
    jobs: Arc<RwLock<HashMap<String, CronJob>>>,
    prompt_tx: mpsc::UnboundedSender<String>,
    next_id: Arc<AtomicU64>,
    project_dir: Option<PathBuf>,
}

impl CronScheduler {
    pub fn new(prompt_tx: mpsc::UnboundedSender<String>, project_dir: Option<PathBuf>) -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
            prompt_tx,
            next_id: Arc::new(AtomicU64::new(1)),
            project_dir,
        }
    }

    /// Start the background scheduler loop. Call once.
    pub fn start(&self) {
        let jobs = self.jobs.clone();
        let tx = self.prompt_tx.clone();
        let project_dir = self.project_dir.clone();

        tokio::spawn(async move {
            // Load durable jobs on startup
            if let Some(ref dir) = project_dir {
                if let Err(e) = Self::load_durable_jobs_inner(&jobs, dir).await {
                    warn!("failed to load durable jobs: {}", e);
                }
            }

            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                Self::tick_inner(&jobs, &tx, project_dir.as_deref()).await;
            }
        });
    }

    pub async fn create_job(
        &self,
        cron_expr: String,
        prompt: String,
        recurring: bool,
        durable: bool,
    ) -> Result<String> {
        // Validate cron expression
        CronExpr::parse(&cron_expr)?;

        let id = format!("cron_{}", self.next_id.fetch_add(1, Ordering::SeqCst));
        let now = chrono::Utc::now().timestamp() as u64;
        let expires_at = if recurring {
            now + 7 * 24 * 3600 // 7 days
        } else {
            now + 365 * 24 * 3600 // 1 year (effectively no expiry for one-shot)
        };

        let job = CronJob {
            id: id.clone(),
            cron_expr,
            prompt,
            recurring,
            durable,
            created_at: now,
            last_fired: None,
            expires_at,
        };

        self.jobs.write().await.insert(id.clone(), job);

        if durable {
            self.save_durable_jobs().await?;
        }

        Ok(id)
    }

    pub async fn delete_job(&self, id: &str) -> Result<bool> {
        let removed = self.jobs.write().await.remove(id).is_some();
        if removed {
            self.save_durable_jobs().await?;
        }
        Ok(removed)
    }

    pub async fn list_jobs(&self) -> Vec<CronJob> {
        self.jobs.read().await.values().cloned().collect()
    }

    async fn tick_inner(
        jobs: &Arc<RwLock<HashMap<String, CronJob>>>,
        tx: &mpsc::UnboundedSender<String>,
        project_dir: Option<&Path>,
    ) {
        let now = Local::now();
        let now_ts = now.timestamp() as u64;

        let mut to_fire = Vec::new();
        let mut to_remove = Vec::new();

        {
            let jobs_read = jobs.read().await;
            for (id, job) in jobs_read.iter() {
                // Check expiry
                if now_ts > job.expires_at {
                    to_remove.push(id.clone());
                    continue;
                }

                // Check if already fired this minute
                if let Some(last) = job.last_fired {
                    let last_dt = chrono::DateTime::from_timestamp(last as i64, 0)
                        .map(|dt| dt.with_timezone(&Local));
                    if let Some(ldt) = last_dt {
                        if ldt.minute() == now.minute()
                            && ldt.hour() == now.hour()
                            && ldt.day() == now.day()
                        {
                            continue; // Already fired this minute
                        }
                    }
                }

                // Check cron match
                if let Ok(expr) = CronExpr::parse(&job.cron_expr) {
                    if expr.matches(&now) {
                        to_fire.push((id.clone(), job.prompt.clone(), job.recurring));
                    }
                }
            }
        }

        // Fire matched jobs
        for (id, prompt, _recurring) in &to_fire {
            info!("cron job {} triggered", id);
            tx.send(prompt.clone()).ok();
        }

        // Update state
        {
            let mut jobs_write = jobs.write().await;
            for (id, _, recurring) in &to_fire {
                if !recurring {
                    jobs_write.remove(id);
                } else if let Some(job) = jobs_write.get_mut(id) {
                    job.last_fired = Some(now_ts);
                }
            }
            for id in &to_remove {
                jobs_write.remove(id);
            }
        }

        // Save if any durable jobs changed
        if !to_fire.is_empty() || !to_remove.is_empty() {
            if let Some(dir) = project_dir {
                let _ = Self::save_durable_jobs_inner(jobs, dir).await;
            }
        }
    }

    async fn save_durable_jobs(&self) -> Result<()> {
        if let Some(ref dir) = self.project_dir {
            Self::save_durable_jobs_inner(&self.jobs, dir).await
        } else {
            Ok(())
        }
    }

    async fn save_durable_jobs_inner(
        jobs: &Arc<RwLock<HashMap<String, CronJob>>>,
        project_dir: &Path,
    ) -> Result<()> {
        let jobs_read = jobs.read().await;
        let durable: Vec<&CronJob> = jobs_read.values().filter(|j| j.durable).collect();
        let json = serde_json::to_string_pretty(&serde_json::json!({ "jobs": durable }))?;
        let path = project_dir.join(".venus").join("scheduled_tasks.json");
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&path, json).await?;
        Ok(())
    }

    async fn load_durable_jobs_inner(
        jobs: &Arc<RwLock<HashMap<String, CronJob>>>,
        project_dir: &Path,
    ) -> Result<()> {
        let path = project_dir.join(".venus").join("scheduled_tasks.json");
        if !path.exists() {
            return Ok(());
        }
        let content = tokio::fs::read_to_string(&path).await?;
        let data: serde_json::Value = serde_json::from_str(&content)?;
        if let Some(arr) = data.get("jobs").and_then(|v| v.as_array()) {
            let mut jobs_write = jobs.write().await;
            for item in arr {
                if let Ok(job) = serde_json::from_value::<CronJob>(item.clone()) {
                    jobs_write.insert(job.id.clone(), job);
                }
            }
        }
        Ok(())
    }
}

// ---- Tests ----

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cron_parse_wildcard() {
        let expr = CronExpr::parse("* * * * *").unwrap();
        assert_eq!(expr.minutes.len(), 60);
        assert_eq!(expr.hours.len(), 24);
    }

    #[test]
    fn test_cron_parse_specific() {
        let expr = CronExpr::parse("30 14 * * *").unwrap();
        assert_eq!(expr.minutes, vec![30]);
        assert_eq!(expr.hours, vec![14]);
    }

    #[test]
    fn test_cron_parse_step() {
        let expr = CronExpr::parse("*/5 * * * *").unwrap();
        assert_eq!(
            expr.minutes,
            vec![0, 5, 10, 15, 20, 25, 30, 35, 40, 45, 50, 55]
        );
    }

    #[test]
    fn test_cron_parse_range() {
        let expr = CronExpr::parse("0 9-17 * * 1-5").unwrap();
        assert_eq!(expr.hours, vec![9, 10, 11, 12, 13, 14, 15, 16, 17]);
        assert_eq!(expr.days_of_week, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_cron_parse_list() {
        let expr = CronExpr::parse("0,30 * * * *").unwrap();
        assert_eq!(expr.minutes, vec![0, 30]);
    }

    #[test]
    fn test_cron_invalid() {
        assert!(CronExpr::parse("* *").is_err());
        assert!(CronExpr::parse("60 * * * *").is_err());
        assert!(CronExpr::parse("* 25 * * *").is_err());
    }
}
