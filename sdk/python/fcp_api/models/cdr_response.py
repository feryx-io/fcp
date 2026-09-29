from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="CdrResponse")



@_attrs_define
class CdrResponse:
    """ CDR response model.

        Attributes:
            cdr_id (str): The record's own identity (`docs/CDR_PIPELINE.md` §5).
            kind (str): What the record is a record of: `call`, `queue`, `conference`, `gateway`.
            call_id (str):
            tenant_id (str):
            sip_call_id (str):
            from_uri (str):
            to_uri (str):
            start_time (datetime.datetime):
            disposition (str):
            hangup_cause (str):
            media_anchored (bool):
            from_display_name (None | str | Unset):
            to_display_name (None | str | Unset):
            answer_time (datetime.datetime | None | Unset):
            end_time (datetime.datetime | None | Unset):
            setup_duration_ms (int | None | Unset):
            call_duration_ms (int | None | Unset):
            total_duration_ms (int | None | Unset):
            hangup_party (None | str | Unset):
            sip_status_code (int | None | Unset):
            sip_reason (None | str | Unset):
            direction (None | str | Unset): `inbound`, `outbound` or `internal` once the pipeline has enriched
                the record; absent until then.
            node_id (None | str | Unset):
            region (None | str | Unset):
            correlation_id (None | str | Unset):
            trunk_name (None | str | Unset):
            trunk_id (None | str | Unset): Which trunk carried the call (`docs/TENANT_TRUNKS.md` §4): two
                tenants may both have a "carrier-a".
            codec_a_leg (None | str | Unset):
            codec_b_leg (None | str | Unset):
            rtp_packets_sent (int | None | Unset):
            rtp_packets_received (int | None | Unset):
            rtp_bytes_sent (int | None | Unset):
            rtp_bytes_received (int | None | Unset):
            rtp_packets_lost (int | None | Unset):
            rtp_jitter_ms (float | None | Unset):
            recording_url (None | str | Unset):
            recording_consent (None | str | Unset): `announced`, `failed`, `not_required` or `refused` when the call was
                to be recorded; absent otherwise.
            cost (float | None | Unset):
            currency (None | str | Unset):
            rate_plan_id (None | str | Unset):
            matched_prefix (None | str | Unset):
            billable_seconds (int | None | Unset):
            rated_at (datetime.datetime | None | Unset):
            created_at (datetime.datetime | None | Unset):
            custom_data (Any | Unset):
     """

    cdr_id: str
    kind: str
    call_id: str
    tenant_id: str
    sip_call_id: str
    from_uri: str
    to_uri: str
    start_time: datetime.datetime
    disposition: str
    hangup_cause: str
    media_anchored: bool
    from_display_name: None | str | Unset = UNSET
    to_display_name: None | str | Unset = UNSET
    answer_time: datetime.datetime | None | Unset = UNSET
    end_time: datetime.datetime | None | Unset = UNSET
    setup_duration_ms: int | None | Unset = UNSET
    call_duration_ms: int | None | Unset = UNSET
    total_duration_ms: int | None | Unset = UNSET
    hangup_party: None | str | Unset = UNSET
    sip_status_code: int | None | Unset = UNSET
    sip_reason: None | str | Unset = UNSET
    direction: None | str | Unset = UNSET
    node_id: None | str | Unset = UNSET
    region: None | str | Unset = UNSET
    correlation_id: None | str | Unset = UNSET
    trunk_name: None | str | Unset = UNSET
    trunk_id: None | str | Unset = UNSET
    codec_a_leg: None | str | Unset = UNSET
    codec_b_leg: None | str | Unset = UNSET
    rtp_packets_sent: int | None | Unset = UNSET
    rtp_packets_received: int | None | Unset = UNSET
    rtp_bytes_sent: int | None | Unset = UNSET
    rtp_bytes_received: int | None | Unset = UNSET
    rtp_packets_lost: int | None | Unset = UNSET
    rtp_jitter_ms: float | None | Unset = UNSET
    recording_url: None | str | Unset = UNSET
    recording_consent: None | str | Unset = UNSET
    cost: float | None | Unset = UNSET
    currency: None | str | Unset = UNSET
    rate_plan_id: None | str | Unset = UNSET
    matched_prefix: None | str | Unset = UNSET
    billable_seconds: int | None | Unset = UNSET
    rated_at: datetime.datetime | None | Unset = UNSET
    created_at: datetime.datetime | None | Unset = UNSET
    custom_data: Any | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        cdr_id = self.cdr_id

        kind = self.kind

        call_id = self.call_id

        tenant_id = self.tenant_id

        sip_call_id = self.sip_call_id

        from_uri = self.from_uri

        to_uri = self.to_uri

        start_time = self.start_time.isoformat()

        disposition = self.disposition

        hangup_cause = self.hangup_cause

        media_anchored = self.media_anchored

        from_display_name: None | str | Unset
        if isinstance(self.from_display_name, Unset):
            from_display_name = UNSET
        else:
            from_display_name = self.from_display_name

        to_display_name: None | str | Unset
        if isinstance(self.to_display_name, Unset):
            to_display_name = UNSET
        else:
            to_display_name = self.to_display_name

        answer_time: None | str | Unset
        if isinstance(self.answer_time, Unset):
            answer_time = UNSET
        elif isinstance(self.answer_time, datetime.datetime):
            answer_time = self.answer_time.isoformat()
        else:
            answer_time = self.answer_time

        end_time: None | str | Unset
        if isinstance(self.end_time, Unset):
            end_time = UNSET
        elif isinstance(self.end_time, datetime.datetime):
            end_time = self.end_time.isoformat()
        else:
            end_time = self.end_time

        setup_duration_ms: int | None | Unset
        if isinstance(self.setup_duration_ms, Unset):
            setup_duration_ms = UNSET
        else:
            setup_duration_ms = self.setup_duration_ms

        call_duration_ms: int | None | Unset
        if isinstance(self.call_duration_ms, Unset):
            call_duration_ms = UNSET
        else:
            call_duration_ms = self.call_duration_ms

        total_duration_ms: int | None | Unset
        if isinstance(self.total_duration_ms, Unset):
            total_duration_ms = UNSET
        else:
            total_duration_ms = self.total_duration_ms

        hangup_party: None | str | Unset
        if isinstance(self.hangup_party, Unset):
            hangup_party = UNSET
        else:
            hangup_party = self.hangup_party

        sip_status_code: int | None | Unset
        if isinstance(self.sip_status_code, Unset):
            sip_status_code = UNSET
        else:
            sip_status_code = self.sip_status_code

        sip_reason: None | str | Unset
        if isinstance(self.sip_reason, Unset):
            sip_reason = UNSET
        else:
            sip_reason = self.sip_reason

        direction: None | str | Unset
        if isinstance(self.direction, Unset):
            direction = UNSET
        else:
            direction = self.direction

        node_id: None | str | Unset
        if isinstance(self.node_id, Unset):
            node_id = UNSET
        else:
            node_id = self.node_id

        region: None | str | Unset
        if isinstance(self.region, Unset):
            region = UNSET
        else:
            region = self.region

        correlation_id: None | str | Unset
        if isinstance(self.correlation_id, Unset):
            correlation_id = UNSET
        else:
            correlation_id = self.correlation_id

        trunk_name: None | str | Unset
        if isinstance(self.trunk_name, Unset):
            trunk_name = UNSET
        else:
            trunk_name = self.trunk_name

        trunk_id: None | str | Unset
        if isinstance(self.trunk_id, Unset):
            trunk_id = UNSET
        else:
            trunk_id = self.trunk_id

        codec_a_leg: None | str | Unset
        if isinstance(self.codec_a_leg, Unset):
            codec_a_leg = UNSET
        else:
            codec_a_leg = self.codec_a_leg

        codec_b_leg: None | str | Unset
        if isinstance(self.codec_b_leg, Unset):
            codec_b_leg = UNSET
        else:
            codec_b_leg = self.codec_b_leg

        rtp_packets_sent: int | None | Unset
        if isinstance(self.rtp_packets_sent, Unset):
            rtp_packets_sent = UNSET
        else:
            rtp_packets_sent = self.rtp_packets_sent

        rtp_packets_received: int | None | Unset
        if isinstance(self.rtp_packets_received, Unset):
            rtp_packets_received = UNSET
        else:
            rtp_packets_received = self.rtp_packets_received

        rtp_bytes_sent: int | None | Unset
        if isinstance(self.rtp_bytes_sent, Unset):
            rtp_bytes_sent = UNSET
        else:
            rtp_bytes_sent = self.rtp_bytes_sent

        rtp_bytes_received: int | None | Unset
        if isinstance(self.rtp_bytes_received, Unset):
            rtp_bytes_received = UNSET
        else:
            rtp_bytes_received = self.rtp_bytes_received

        rtp_packets_lost: int | None | Unset
        if isinstance(self.rtp_packets_lost, Unset):
            rtp_packets_lost = UNSET
        else:
            rtp_packets_lost = self.rtp_packets_lost

        rtp_jitter_ms: float | None | Unset
        if isinstance(self.rtp_jitter_ms, Unset):
            rtp_jitter_ms = UNSET
        else:
            rtp_jitter_ms = self.rtp_jitter_ms

        recording_url: None | str | Unset
        if isinstance(self.recording_url, Unset):
            recording_url = UNSET
        else:
            recording_url = self.recording_url

        recording_consent: None | str | Unset
        if isinstance(self.recording_consent, Unset):
            recording_consent = UNSET
        else:
            recording_consent = self.recording_consent

        cost: float | None | Unset
        if isinstance(self.cost, Unset):
            cost = UNSET
        else:
            cost = self.cost

        currency: None | str | Unset
        if isinstance(self.currency, Unset):
            currency = UNSET
        else:
            currency = self.currency

        rate_plan_id: None | str | Unset
        if isinstance(self.rate_plan_id, Unset):
            rate_plan_id = UNSET
        else:
            rate_plan_id = self.rate_plan_id

        matched_prefix: None | str | Unset
        if isinstance(self.matched_prefix, Unset):
            matched_prefix = UNSET
        else:
            matched_prefix = self.matched_prefix

        billable_seconds: int | None | Unset
        if isinstance(self.billable_seconds, Unset):
            billable_seconds = UNSET
        else:
            billable_seconds = self.billable_seconds

        rated_at: None | str | Unset
        if isinstance(self.rated_at, Unset):
            rated_at = UNSET
        elif isinstance(self.rated_at, datetime.datetime):
            rated_at = self.rated_at.isoformat()
        else:
            rated_at = self.rated_at

        created_at: None | str | Unset
        if isinstance(self.created_at, Unset):
            created_at = UNSET
        elif isinstance(self.created_at, datetime.datetime):
            created_at = self.created_at.isoformat()
        else:
            created_at = self.created_at

        custom_data = self.custom_data


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "cdr_id": cdr_id,
            "kind": kind,
            "call_id": call_id,
            "tenant_id": tenant_id,
            "sip_call_id": sip_call_id,
            "from_uri": from_uri,
            "to_uri": to_uri,
            "start_time": start_time,
            "disposition": disposition,
            "hangup_cause": hangup_cause,
            "media_anchored": media_anchored,
        })
        if from_display_name is not UNSET:
            field_dict["from_display_name"] = from_display_name
        if to_display_name is not UNSET:
            field_dict["to_display_name"] = to_display_name
        if answer_time is not UNSET:
            field_dict["answer_time"] = answer_time
        if end_time is not UNSET:
            field_dict["end_time"] = end_time
        if setup_duration_ms is not UNSET:
            field_dict["setup_duration_ms"] = setup_duration_ms
        if call_duration_ms is not UNSET:
            field_dict["call_duration_ms"] = call_duration_ms
        if total_duration_ms is not UNSET:
            field_dict["total_duration_ms"] = total_duration_ms
        if hangup_party is not UNSET:
            field_dict["hangup_party"] = hangup_party
        if sip_status_code is not UNSET:
            field_dict["sip_status_code"] = sip_status_code
        if sip_reason is not UNSET:
            field_dict["sip_reason"] = sip_reason
        if direction is not UNSET:
            field_dict["direction"] = direction
        if node_id is not UNSET:
            field_dict["node_id"] = node_id
        if region is not UNSET:
            field_dict["region"] = region
        if correlation_id is not UNSET:
            field_dict["correlation_id"] = correlation_id
        if trunk_name is not UNSET:
            field_dict["trunk_name"] = trunk_name
        if trunk_id is not UNSET:
            field_dict["trunk_id"] = trunk_id
        if codec_a_leg is not UNSET:
            field_dict["codec_a_leg"] = codec_a_leg
        if codec_b_leg is not UNSET:
            field_dict["codec_b_leg"] = codec_b_leg
        if rtp_packets_sent is not UNSET:
            field_dict["rtp_packets_sent"] = rtp_packets_sent
        if rtp_packets_received is not UNSET:
            field_dict["rtp_packets_received"] = rtp_packets_received
        if rtp_bytes_sent is not UNSET:
            field_dict["rtp_bytes_sent"] = rtp_bytes_sent
        if rtp_bytes_received is not UNSET:
            field_dict["rtp_bytes_received"] = rtp_bytes_received
        if rtp_packets_lost is not UNSET:
            field_dict["rtp_packets_lost"] = rtp_packets_lost
        if rtp_jitter_ms is not UNSET:
            field_dict["rtp_jitter_ms"] = rtp_jitter_ms
        if recording_url is not UNSET:
            field_dict["recording_url"] = recording_url
        if recording_consent is not UNSET:
            field_dict["recording_consent"] = recording_consent
        if cost is not UNSET:
            field_dict["cost"] = cost
        if currency is not UNSET:
            field_dict["currency"] = currency
        if rate_plan_id is not UNSET:
            field_dict["rate_plan_id"] = rate_plan_id
        if matched_prefix is not UNSET:
            field_dict["matched_prefix"] = matched_prefix
        if billable_seconds is not UNSET:
            field_dict["billable_seconds"] = billable_seconds
        if rated_at is not UNSET:
            field_dict["rated_at"] = rated_at
        if created_at is not UNSET:
            field_dict["created_at"] = created_at
        if custom_data is not UNSET:
            field_dict["custom_data"] = custom_data

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        cdr_id = d.pop("cdr_id")

        kind = d.pop("kind")

        call_id = d.pop("call_id")

        tenant_id = d.pop("tenant_id")

        sip_call_id = d.pop("sip_call_id")

        from_uri = d.pop("from_uri")

        to_uri = d.pop("to_uri")

        start_time = datetime.datetime.fromisoformat(d.pop("start_time"))




        disposition = d.pop("disposition")

        hangup_cause = d.pop("hangup_cause")

        media_anchored = d.pop("media_anchored")

        def _parse_from_display_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        from_display_name = _parse_from_display_name(d.pop("from_display_name", UNSET))


        def _parse_to_display_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        to_display_name = _parse_to_display_name(d.pop("to_display_name", UNSET))


        def _parse_answer_time(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                answer_time_type_0 = datetime.datetime.fromisoformat(data)



                return answer_time_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        answer_time = _parse_answer_time(d.pop("answer_time", UNSET))


        def _parse_end_time(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                end_time_type_0 = datetime.datetime.fromisoformat(data)



                return end_time_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        end_time = _parse_end_time(d.pop("end_time", UNSET))


        def _parse_setup_duration_ms(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        setup_duration_ms = _parse_setup_duration_ms(d.pop("setup_duration_ms", UNSET))


        def _parse_call_duration_ms(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        call_duration_ms = _parse_call_duration_ms(d.pop("call_duration_ms", UNSET))


        def _parse_total_duration_ms(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        total_duration_ms = _parse_total_duration_ms(d.pop("total_duration_ms", UNSET))


        def _parse_hangup_party(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        hangup_party = _parse_hangup_party(d.pop("hangup_party", UNSET))


        def _parse_sip_status_code(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        sip_status_code = _parse_sip_status_code(d.pop("sip_status_code", UNSET))


        def _parse_sip_reason(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        sip_reason = _parse_sip_reason(d.pop("sip_reason", UNSET))


        def _parse_direction(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        direction = _parse_direction(d.pop("direction", UNSET))


        def _parse_node_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        node_id = _parse_node_id(d.pop("node_id", UNSET))


        def _parse_region(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        region = _parse_region(d.pop("region", UNSET))


        def _parse_correlation_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        correlation_id = _parse_correlation_id(d.pop("correlation_id", UNSET))


        def _parse_trunk_name(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        trunk_name = _parse_trunk_name(d.pop("trunk_name", UNSET))


        def _parse_trunk_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        trunk_id = _parse_trunk_id(d.pop("trunk_id", UNSET))


        def _parse_codec_a_leg(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        codec_a_leg = _parse_codec_a_leg(d.pop("codec_a_leg", UNSET))


        def _parse_codec_b_leg(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        codec_b_leg = _parse_codec_b_leg(d.pop("codec_b_leg", UNSET))


        def _parse_rtp_packets_sent(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        rtp_packets_sent = _parse_rtp_packets_sent(d.pop("rtp_packets_sent", UNSET))


        def _parse_rtp_packets_received(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        rtp_packets_received = _parse_rtp_packets_received(d.pop("rtp_packets_received", UNSET))


        def _parse_rtp_bytes_sent(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        rtp_bytes_sent = _parse_rtp_bytes_sent(d.pop("rtp_bytes_sent", UNSET))


        def _parse_rtp_bytes_received(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        rtp_bytes_received = _parse_rtp_bytes_received(d.pop("rtp_bytes_received", UNSET))


        def _parse_rtp_packets_lost(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        rtp_packets_lost = _parse_rtp_packets_lost(d.pop("rtp_packets_lost", UNSET))


        def _parse_rtp_jitter_ms(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        rtp_jitter_ms = _parse_rtp_jitter_ms(d.pop("rtp_jitter_ms", UNSET))


        def _parse_recording_url(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        recording_url = _parse_recording_url(d.pop("recording_url", UNSET))


        def _parse_recording_consent(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        recording_consent = _parse_recording_consent(d.pop("recording_consent", UNSET))


        def _parse_cost(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        cost = _parse_cost(d.pop("cost", UNSET))


        def _parse_currency(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        currency = _parse_currency(d.pop("currency", UNSET))


        def _parse_rate_plan_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        rate_plan_id = _parse_rate_plan_id(d.pop("rate_plan_id", UNSET))


        def _parse_matched_prefix(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        matched_prefix = _parse_matched_prefix(d.pop("matched_prefix", UNSET))


        def _parse_billable_seconds(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        billable_seconds = _parse_billable_seconds(d.pop("billable_seconds", UNSET))


        def _parse_rated_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                rated_at_type_0 = datetime.datetime.fromisoformat(data)



                return rated_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        rated_at = _parse_rated_at(d.pop("rated_at", UNSET))


        def _parse_created_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                created_at_type_0 = datetime.datetime.fromisoformat(data)



                return created_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        created_at = _parse_created_at(d.pop("created_at", UNSET))


        custom_data = d.pop("custom_data", UNSET)

        cdr_response = cls(
            cdr_id=cdr_id,
            kind=kind,
            call_id=call_id,
            tenant_id=tenant_id,
            sip_call_id=sip_call_id,
            from_uri=from_uri,
            to_uri=to_uri,
            start_time=start_time,
            disposition=disposition,
            hangup_cause=hangup_cause,
            media_anchored=media_anchored,
            from_display_name=from_display_name,
            to_display_name=to_display_name,
            answer_time=answer_time,
            end_time=end_time,
            setup_duration_ms=setup_duration_ms,
            call_duration_ms=call_duration_ms,
            total_duration_ms=total_duration_ms,
            hangup_party=hangup_party,
            sip_status_code=sip_status_code,
            sip_reason=sip_reason,
            direction=direction,
            node_id=node_id,
            region=region,
            correlation_id=correlation_id,
            trunk_name=trunk_name,
            trunk_id=trunk_id,
            codec_a_leg=codec_a_leg,
            codec_b_leg=codec_b_leg,
            rtp_packets_sent=rtp_packets_sent,
            rtp_packets_received=rtp_packets_received,
            rtp_bytes_sent=rtp_bytes_sent,
            rtp_bytes_received=rtp_bytes_received,
            rtp_packets_lost=rtp_packets_lost,
            rtp_jitter_ms=rtp_jitter_ms,
            recording_url=recording_url,
            recording_consent=recording_consent,
            cost=cost,
            currency=currency,
            rate_plan_id=rate_plan_id,
            matched_prefix=matched_prefix,
            billable_seconds=billable_seconds,
            rated_at=rated_at,
            created_at=created_at,
            custom_data=custom_data,
        )


        cdr_response.additional_properties = d
        return cdr_response

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
