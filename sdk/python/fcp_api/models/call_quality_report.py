from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="CallQualityReport")



@_attrs_define
class CallQualityReport:
    """ A call's final figures, as the app measured them.

        Attributes:
            direction (str): `incoming` or `outgoing`, from the phone's side.
            codec (str): The codec (`pcmu`, `opus`, …).
            packets_sent (int):
            packets_received (int):
            packets_expected (int): The far end's packets its sequence numbers say were sent, and those
                that never arrived.
            packets_lost (int):
            jitter_ms (float): The phone's jitter estimate on what it received, at the end, in ms.
            mos (float): The estimated score (E-model MOS, 1 to 4.5) of what the user heard,
                and of what the far end heard once it reported.
            srtp (bool | Unset):
            duration_seconds (int | None | Unset): Talk time, as the phone counted it.
            rtt_ms (float | None | Unset): The round trip from RTCP, when it was measured.
            far_loss_pct (float | None | Unset): What the far end last reported over RTCP: its loss in its last
                interval in percent, its jitter in ms, its losses in all.
            far_jitter_ms (float | None | Unset):
            far_lost (int | None | Unset):
            far_mos (float | None | Unset):
            local_rtp (None | str | Unset): Where media flowed between, as the phone saw it (`address:port`).
            remote_rtp (None | str | Unset):
     """

    direction: str
    codec: str
    packets_sent: int
    packets_received: int
    packets_expected: int
    packets_lost: int
    jitter_ms: float
    mos: float
    srtp: bool | Unset = UNSET
    duration_seconds: int | None | Unset = UNSET
    rtt_ms: float | None | Unset = UNSET
    far_loss_pct: float | None | Unset = UNSET
    far_jitter_ms: float | None | Unset = UNSET
    far_lost: int | None | Unset = UNSET
    far_mos: float | None | Unset = UNSET
    local_rtp: None | str | Unset = UNSET
    remote_rtp: None | str | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        direction = self.direction

        codec = self.codec

        packets_sent = self.packets_sent

        packets_received = self.packets_received

        packets_expected = self.packets_expected

        packets_lost = self.packets_lost

        jitter_ms = self.jitter_ms

        mos = self.mos

        srtp = self.srtp

        duration_seconds: int | None | Unset
        if isinstance(self.duration_seconds, Unset):
            duration_seconds = UNSET
        else:
            duration_seconds = self.duration_seconds

        rtt_ms: float | None | Unset
        if isinstance(self.rtt_ms, Unset):
            rtt_ms = UNSET
        else:
            rtt_ms = self.rtt_ms

        far_loss_pct: float | None | Unset
        if isinstance(self.far_loss_pct, Unset):
            far_loss_pct = UNSET
        else:
            far_loss_pct = self.far_loss_pct

        far_jitter_ms: float | None | Unset
        if isinstance(self.far_jitter_ms, Unset):
            far_jitter_ms = UNSET
        else:
            far_jitter_ms = self.far_jitter_ms

        far_lost: int | None | Unset
        if isinstance(self.far_lost, Unset):
            far_lost = UNSET
        else:
            far_lost = self.far_lost

        far_mos: float | None | Unset
        if isinstance(self.far_mos, Unset):
            far_mos = UNSET
        else:
            far_mos = self.far_mos

        local_rtp: None | str | Unset
        if isinstance(self.local_rtp, Unset):
            local_rtp = UNSET
        else:
            local_rtp = self.local_rtp

        remote_rtp: None | str | Unset
        if isinstance(self.remote_rtp, Unset):
            remote_rtp = UNSET
        else:
            remote_rtp = self.remote_rtp


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "direction": direction,
            "codec": codec,
            "packets_sent": packets_sent,
            "packets_received": packets_received,
            "packets_expected": packets_expected,
            "packets_lost": packets_lost,
            "jitter_ms": jitter_ms,
            "mos": mos,
        })
        if srtp is not UNSET:
            field_dict["srtp"] = srtp
        if duration_seconds is not UNSET:
            field_dict["duration_seconds"] = duration_seconds
        if rtt_ms is not UNSET:
            field_dict["rtt_ms"] = rtt_ms
        if far_loss_pct is not UNSET:
            field_dict["far_loss_pct"] = far_loss_pct
        if far_jitter_ms is not UNSET:
            field_dict["far_jitter_ms"] = far_jitter_ms
        if far_lost is not UNSET:
            field_dict["far_lost"] = far_lost
        if far_mos is not UNSET:
            field_dict["far_mos"] = far_mos
        if local_rtp is not UNSET:
            field_dict["local_rtp"] = local_rtp
        if remote_rtp is not UNSET:
            field_dict["remote_rtp"] = remote_rtp

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        direction = d.pop("direction")

        codec = d.pop("codec")

        packets_sent = d.pop("packets_sent")

        packets_received = d.pop("packets_received")

        packets_expected = d.pop("packets_expected")

        packets_lost = d.pop("packets_lost")

        jitter_ms = d.pop("jitter_ms")

        mos = d.pop("mos")

        srtp = d.pop("srtp", UNSET)

        def _parse_duration_seconds(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        duration_seconds = _parse_duration_seconds(d.pop("duration_seconds", UNSET))


        def _parse_rtt_ms(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        rtt_ms = _parse_rtt_ms(d.pop("rtt_ms", UNSET))


        def _parse_far_loss_pct(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        far_loss_pct = _parse_far_loss_pct(d.pop("far_loss_pct", UNSET))


        def _parse_far_jitter_ms(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        far_jitter_ms = _parse_far_jitter_ms(d.pop("far_jitter_ms", UNSET))


        def _parse_far_lost(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        far_lost = _parse_far_lost(d.pop("far_lost", UNSET))


        def _parse_far_mos(data: object) -> float | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(float | None | Unset, data)

        far_mos = _parse_far_mos(d.pop("far_mos", UNSET))


        def _parse_local_rtp(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        local_rtp = _parse_local_rtp(d.pop("local_rtp", UNSET))


        def _parse_remote_rtp(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        remote_rtp = _parse_remote_rtp(d.pop("remote_rtp", UNSET))


        call_quality_report = cls(
            direction=direction,
            codec=codec,
            packets_sent=packets_sent,
            packets_received=packets_received,
            packets_expected=packets_expected,
            packets_lost=packets_lost,
            jitter_ms=jitter_ms,
            mos=mos,
            srtp=srtp,
            duration_seconds=duration_seconds,
            rtt_ms=rtt_ms,
            far_loss_pct=far_loss_pct,
            far_jitter_ms=far_jitter_ms,
            far_lost=far_lost,
            far_mos=far_mos,
            local_rtp=local_rtp,
            remote_rtp=remote_rtp,
        )

        return call_quality_report

