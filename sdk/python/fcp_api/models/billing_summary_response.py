from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast

if TYPE_CHECKING:
  from ..models.balance_response import BalanceResponse
  from ..models.resource_counts import ResourceCounts
  from ..models.usage_response import UsageResponse





T = TypeVar("T", bound="BillingSummaryResponse")



@_attrs_define
class BillingSummaryResponse:
    """ Billing summary response.

        Attributes:
            period_start (int):
            period_end (int):
            usage (UsageResponse): Usage aggregation response.
            resource_counts (ResourceCounts): Counts of provisioned resources for the tenant.
            balance (BalanceResponse | None | Unset):
     """

    period_start: int
    period_end: int
    usage: UsageResponse
    resource_counts: ResourceCounts
    balance: BalanceResponse | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.balance_response import BalanceResponse # noqa: PLC0415
        from ..models.resource_counts import ResourceCounts # noqa: PLC0415
        from ..models.usage_response import UsageResponse # noqa: PLC0415
        period_start = self.period_start

        period_end = self.period_end

        usage = self.usage.to_dict()

        resource_counts = self.resource_counts.to_dict()

        balance: dict[str, Any] | None | Unset
        if isinstance(self.balance, Unset):
            balance = UNSET
        elif isinstance(self.balance, BalanceResponse):
            balance = self.balance.to_dict()
        else:
            balance = self.balance


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "period_start": period_start,
            "period_end": period_end,
            "usage": usage,
            "resource_counts": resource_counts,
        })
        if balance is not UNSET:
            field_dict["balance"] = balance

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.balance_response import BalanceResponse # noqa: PLC0415
        from ..models.resource_counts import ResourceCounts # noqa: PLC0415
        from ..models.usage_response import UsageResponse # noqa: PLC0415
        d = dict(src_dict)
        period_start = d.pop("period_start")

        period_end = d.pop("period_end")

        usage = UsageResponse.from_dict(d.pop("usage"))




        resource_counts = ResourceCounts.from_dict(d.pop("resource_counts"))




        def _parse_balance(data: object) -> BalanceResponse | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                balance_type_0 = BalanceResponse.from_dict(data)



                return balance_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(BalanceResponse | None | Unset, data)

        balance = _parse_balance(d.pop("balance", UNSET))


        billing_summary_response = cls(
            period_start=period_start,
            period_end=period_end,
            usage=usage,
            resource_counts=resource_counts,
            balance=balance,
        )


        billing_summary_response.additional_properties = d
        return billing_summary_response

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
